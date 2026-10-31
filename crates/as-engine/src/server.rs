//! The engine process: reads requests from stdin, runs one job at a time on
//! the GPU (models stay loaded between jobs), streams replies to stdout.
//!
//! It owns no state on disk. Everything a job produces goes back to the app,
//! which stores it as it arrives — so the engine can be killed at any moment
//! and the job resumes from the app's last checkpoint.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use as_analysis::{
    AnalysisError, BodyModels, DETECT_BATCH, FaceChunk, FaceParams, Faces, FastModels, FastParams, Look, PERSON_INPUT,
    PeopleParams, Search, anime, bodies, detect_looks, face_tracks, fast, fast_face_tracks, frame_sizes, shot_signals,
    signals, tracks,
};
use as_core::{CancelToken, FrameIndex, FrameRange};
use as_infer::{Device, Runtime};
use as_media::{Source, Tools, frames};
use as_models::ModelStore;
use as_vision::{ArcFace, Ccip, Fit, Norm, PersonReid, PpDet, SFace, Scrfd, TransNet, Yolo, YuNet};
use parking_lot::Mutex;

use crate::protocol::{Blob, EmbedJob, FacesJob, ModelDevice, Reply, Request, ShotsJob, Sprite};

/// Faces per embedding call.
const EMBED_BATCH: usize = 32;
/// People per appearance (ReID) call.
const REID_BATCH: usize = 32;
/// Fast Scan: frames per detector call — YuNet's, or on anime the head and
/// person detectors' (it looks at one frame at a time, and decides from
/// what it found where to look next) — and faces per SFace call.
const FAST_DETECT_BATCH: usize = 1;
const FAST_EMBED_BATCH: usize = 16;
/// Crops per CCIP call (a 384² crop is ~12× a face's): one session embeds
/// heads and implied heads alike.
pub const CCIP_BATCH: usize = 4;
/// The anime head detector's letterboxed input (the bench's first-config
/// variant).
const ANIME_HEAD_INPUT: (usize, usize) = (640, 640);
/// The anime person detector's input: it looks at a bigger picture to find
/// characters whose heads are too small for the head detector.
const ANIME_PERSON_INPUT: (usize, usize) = (960, 544);

/// How the engine finds its runtime, models and tools.
#[derive(Clone, Debug)]
pub struct Config {
    /// Folder with the ONNX Runtime library.
    pub ort_dir: PathBuf,
    /// The model store (`AppPaths::models()`).
    pub models_dir: PathBuf,
    /// Bundled ffmpeg folder, if any (otherwise `AUTOSCENE_FFMPEG_DIR` / PATH).
    pub ffmpeg_dir: Option<PathBuf>,
    /// `false` keeps every model on the CPU.
    pub gpu: bool,
    /// Exit after this long without a job, which frees the GPU memory.
    pub idle_exit: Option<Duration>,
}

/// Shared stdout: one reply per line, flushed at once.
#[derive(Clone)]
struct Out(Arc<Mutex<Box<dyn Write + Send>>>);

impl Out {
    fn send(&self, reply: &Reply) {
        let mut line = serde_json::to_string(reply).expect("replies always serialise");
        line.push('\n');
        let mut w = self.0.lock();
        // A broken pipe means the app is gone; the reader thread ends the loop.
        let _ = w.write_all(line.as_bytes()).and_then(|()| w.flush());
    }
}

/// Tell the app where job `id`'s models run (each model's id and device),
/// before its first progress. A model the GPU couldn't take runs on the
/// CPU, much slower: the app shows it with the job, not only in the log.
fn send_devices(out: &Out, id: u64, rt: &Runtime, models: &[(&str, Device)]) {
    let preferred = rt.preferred_device();
    let on_cpu: Vec<&str> =
        models.iter().filter(|(_, d)| *d == Device::Cpu && preferred != Device::Cpu).map(|(m, _)| *m).collect();
    if !on_cpu.is_empty() {
        tracing::warn!(id, ?on_cpu, preferred = ?preferred, "the GPU couldn't take every model; these run on the CPU");
    }
    out.send(&Reply::Devices {
        id,
        preferred: preferred.label().into(),
        models: models.iter().map(|(m, d)| ModelDevice { model: (*m).into(), device: d.label().into() }).collect(),
    });
}

/// Run the engine until stdin closes, a `shutdown` arrives, or it has been
/// idle for `idle_exit`. Returns the process exit code.
pub fn serve(cfg: Config, input: impl BufRead + Send + 'static, output: impl Write + Send + 'static) -> i32 {
    let out = Out(Arc::new(Mutex::new(Box::new(output))));
    let tools = match Tools::locate(cfg.ffmpeg_dir.as_deref()) {
        Ok(t) => t,
        Err(e) => {
            out.send(&Reply::Fatal { message: e.to_string(), detail: e.detail() });
            return 1;
        }
    };
    let rt = match Runtime::load(&cfg.ort_dir, cfg.gpu) {
        Ok(rt) => rt,
        Err(e) => {
            out.send(&Reply::Fatal { message: e.to_string(), detail: e.detail() });
            return 1;
        }
    };
    out.send(&Reply::Ready { device: rt.preferred_device().label().to_string() });

    // Jobs queue up here; cancellation needs no queue and is handled at once.
    let (tx, rx) = mpsc::channel::<(Request, CancelToken)>();
    let cancels: Arc<Mutex<HashMap<u64, CancelToken>>> = Arc::default();
    let reader = {
        let cancels = cancels.clone();
        let out = out.clone();
        std::thread::spawn(move || read_requests(input, tx, cancels, out))
    };

    let mut models = Models {
        store: ModelStore::new(&cfg.models_dir),
        transnet: None,
        scrfd: Vec::new(),
        arcface: None,
        persons: None,
        person_reid: None,
        yunet: None,
        sface: None,
        anime_heads: None,
        anime_persons: None,
        anime_ccip: None,
    };
    loop {
        let next = match cfg.idle_exit {
            Some(idle) => rx.recv_timeout(idle).map_err(|e| e == mpsc::RecvTimeoutError::Timeout),
            None => rx.recv().map_err(|_| false),
        };
        let (job, cancel) = match next {
            Ok(j) => j,
            Err(true) => {
                tracing::info!("idle; exiting to free the GPU");
                break;
            }
            Err(false) => break,
        };
        let id = job.id().expect("only jobs are queued");
        let run = guarded(|| match job {
            Request::Shots(j) => run_shots(&tools, &rt, &mut models, *j, &cancel, &out),
            Request::Faces(j) => run_faces(&tools, &rt, &mut models, *j, &cancel, &out),
            Request::Embed(j) => run_embed(&tools, &rt, &mut models, *j, &cancel, &out),
            Request::Cancel { .. } | Request::Shutdown => Ok(()),
        });
        let result = run.unwrap_or_else(|panic| {
            // A model may have been left mid-batch: the next job loads fresh ones.
            models.unload();
            Err(AnalysisError::Internal(format!("panicked: {panic}")))
        });
        let reply = match result {
            Ok(()) => Reply::Done { id },
            Err(e) => {
                if !e.is_cancelled() {
                    tracing::warn!(id, error = %e.detail(), "job failed");
                }
                Reply::Failed { id, message: e.to_string(), detail: e.detail(), cancelled: e.is_cancelled() }
            }
        };
        cancels.lock().remove(&id);
        out.send(&reply);
    }
    // Pending cancellations of jobs that never ran: nothing to report. The
    // reader thread ends with stdin.
    drop(reader);
    0
}

/// Run one job; what it panicked with, if it did. A bug in one job's pre- or
/// post-processing must not take the engine down: the app would restart it
/// and run the same job into the same panic three times, and the person would
/// only see "the engine stopped". The job fails with its own error instead.
fn guarded(job: impl FnOnce() -> Result<(), AnalysisError>) -> Result<Result<(), AnalysisError>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).map_err(|panic| {
        panic
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into())
    })
}

fn read_requests(
    input: impl BufRead,
    tx: mpsc::Sender<(Request, CancelToken)>,
    cancels: Arc<Mutex<HashMap<u64, CancelToken>>>,
    out: Out,
) {
    // Cancels that arrive before their job (a race the app can't rule out).
    let mut early: HashSet<u64> = HashSet::new();
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Request>(&line) {
            Ok(job @ (Request::Shots(_) | Request::Faces(_) | Request::Embed(_))) => {
                let id = job.id().expect("a job");
                let cancel = CancelToken::new();
                if early.remove(&id) {
                    cancel.cancel();
                }
                cancels.lock().insert(id, cancel.clone());
                if tx.send((job, cancel)).is_err() {
                    break;
                }
            }
            Ok(Request::Cancel { id }) => match cancels.lock().get(&id) {
                Some(c) => c.cancel(),
                None => {
                    early.insert(id);
                }
            },
            Ok(Request::Shutdown) => break,
            Err(e) => {
                tracing::error!(error = %e, "bad request");
                out.send(&Reply::Fatal {
                    message: "The analysis engine got a bad request.".into(),
                    detail: e.to_string(),
                });
            }
        }
    }
    // Leaving: whatever runs now should stop, and the loop ends after it.
    for c in cancels.lock().values() {
        c.cancel();
    }
}

struct Models {
    store: ModelStore,
    transnet: Option<TransNet>,
    /// One face detector per look of the last faces job.
    scrfd: Vec<Scrfd>,
    arcface: Option<ArcFace>,
    persons: Option<PpDet>,
    person_reid: Option<PersonReid>,
    /// Fast Scan's detector, planned for the last media's frame size, and embedder.
    yunet: Option<YuNet>,
    sface: Option<SFace>,
    /// The anime models: head and person detectors planned for the last
    /// media's frame size, and the character embedder (also serves `Embed`
    /// jobs on stills).
    anime_heads: Option<Yolo>,
    anime_persons: Option<Yolo>,
    anime_ccip: Option<Ccip>,
}

/// Which family of models a job keeps loaded; the other family's sessions
/// are dropped so the two passes never hold the GPU at once.
enum Family {
    Live,
    Anime,
}

impl Models {
    fn unload(&mut self) {
        (self.transnet, self.arcface) = (None, None);
        self.scrfd.clear();
        (self.persons, self.person_reid) = (None, None);
        (self.yunet, self.sface) = (None, None);
        (self.anime_heads, self.anime_persons, self.anime_ccip) = (None, None, None);
    }

    /// Drop the sessions of the other family (TransNet — the shot
    /// detector — is neither's and stays).
    fn keep(&mut self, family: Family) {
        match family {
            Family::Live => {
                (self.anime_heads, self.anime_persons, self.anime_ccip) = (None, None, None);
            }
            Family::Anime => {
                self.scrfd.clear();
                (self.arcface, self.persons, self.person_reid) = (None, None, None);
                (self.yunet, self.sface) = (None, None);
            }
        }
    }

    /// The file of model `id`, hashed the first time this engine loads it
    /// (a damaged one loses its marker: the app's next install fetches it).
    fn path(&self, id: &str) -> Result<PathBuf, AnalysisError> {
        self.store.installed(id).map_err(|e| AnalysisError::Internal(e.detail()))?.ok_or_else(|| {
            AnalysisError::Internal(format!("{id} isn't installed (or is damaged) in {}", self.store.dir().display()))
        })
    }

    /// Load the face models, a detector planned for each look's size
    /// (loaded again when a media needs other sizes; one of a size still
    /// needed stays).
    fn load_faces(&mut self, rt: &Runtime, looks: &[Look]) -> Result<(), AnalysisError> {
        self.keep(Family::Live);
        if !self.scrfd.iter().map(Scrfd::frame_size).eq(looks.iter().map(|l| l.size)) {
            let mut old = std::mem::take(&mut self.scrfd);
            old.retain(|d| looks.iter().any(|l| l.size == d.frame_size()));
            let path = self.path(as_models::ids::SCRFD)?;
            for l in looks {
                let detector = match old.iter().position(|d| d.frame_size() == l.size) {
                    Some(i) => old.swap_remove(i),
                    None => Scrfd::load(rt, &path, l.size, DETECT_BATCH)?,
                };
                self.scrfd.push(detector);
            }
        }
        if self.arcface.is_none() {
            self.arcface = Some(ArcFace::load(rt, &self.path(as_models::ids::ARCFACE)?, EMBED_BATCH)?);
        }
        Ok(())
    }

    /// Load Fast Scan's models, the detector planned for `size` frames.
    fn load_fast(&mut self, rt: &Runtime, size: (usize, usize)) -> Result<(), AnalysisError> {
        self.keep(Family::Live);
        if self.yunet.as_ref().is_none_or(|d| d.frame_size() != size) {
            self.yunet = None;
            let path = self.path(as_models::ids::YUNET)?;
            self.yunet = Some(YuNet::load(rt, &path, size, FAST_DETECT_BATCH)?);
        }
        if self.sface.is_none() {
            self.sface = Some(SFace::load(rt, &self.path(as_models::ids::SFACE)?, FAST_EMBED_BATCH)?);
        }
        Ok(())
    }

    /// Load the people models, the detector planned for `size` frames.
    fn load_people(&mut self, rt: &Runtime, size: (usize, usize)) -> Result<(), AnalysisError> {
        self.keep(Family::Live);
        if self.persons.as_ref().is_none_or(|d| d.frame_size() != size) {
            self.persons = None;
            let path = self.path(as_models::ids::PERSONS)?;
            self.persons = Some(PpDet::load(rt, &path, size, PERSON_INPUT, Fit::Pad, Norm::IMAGENET)?);
        }
        if self.person_reid.is_none() {
            let path = self.path(as_models::ids::PERSON_REID)?;
            self.person_reid = Some(PersonReid::load(rt, &path, REID_BATCH, Norm::IMAGENET)?);
        }
        Ok(())
    }

    /// The anime character embedder (CCIP) alone — `Embed` jobs on stills.
    fn load_ccip(&mut self, rt: &Runtime) -> Result<(), AnalysisError> {
        self.keep(Family::Anime);
        if self.anime_ccip.is_none() {
            self.anime_ccip = Some(Ccip::load(rt, &self.path(as_models::ids::ANIME_CCIP)?, CCIP_BATCH)?);
        }
        Ok(())
    }

    /// Load the anime models, the detectors planned for `size` frames,
    /// `batch` at a time (Fast Scan looks at one frame at a time).
    /// The embedder loads first: its golden check against the CPU decides
    /// where everything else lands (a CCIP that fell back means the
    /// detectors might as well run beside it).
    fn load_anime(&mut self, rt: &Runtime, size: (usize, usize), batch: usize) -> Result<(), AnalysisError> {
        self.load_ccip(rt)?;
        let stale = |d: &Yolo| d.frame_size() != size || d.batch() != batch;
        if self.anime_heads.as_ref().is_none_or(stale) {
            self.anime_heads = None;
            let path = self.path(as_models::ids::ANIME_HEAD)?;
            self.anime_heads = Some(Yolo::load(rt, &path, size, ANIME_HEAD_INPUT, batch)?);
        }
        if self.anime_persons.as_ref().is_none_or(stale) {
            self.anime_persons = None;
            let path = self.path(as_models::ids::ANIME_PERSON)?;
            self.anime_persons = Some(Yolo::load(rt, &path, size, ANIME_PERSON_INPUT, batch)?);
        }
        Ok(())
    }

    fn transnet(&mut self, rt: &Runtime) -> Result<&mut TransNet, AnalysisError> {
        if self.transnet.is_none() {
            let path = self.path(as_models::ids::TRANSNET)?;
            self.transnet = Some(TransNet::load(rt, &path)?);
        }
        Ok(self.transnet.as_mut().expect("just loaded"))
    }
}

fn run_shots(
    tools: &Tools,
    rt: &Runtime,
    models: &mut Models,
    job: ShotsJob,
    cancel: &CancelToken,
    out: &Out,
) -> Result<(), AnalysisError> {
    let net = models.transnet(rt)?;
    let index = FrameIndex::from_bytes(&job.index.0).map_err(|e| AnalysisError::Internal(e.to_string()))?;
    let started = Instant::now();
    let (id, file) = (job.id, job.path.file_name().unwrap_or_default());
    send_devices(out, id, rt, &[(as_models::ids::TRANSNET, net.device())]);
    tracing::info!(id, ?file, resume = job.resume, device = ?net.device(), "finding shots");
    shot_signals(
        tools,
        Source { path: Path::new(&job.path), info: &job.info, index: &index },
        net,
        job.resume,
        job.chunk,
        cancel,
        |fraction| out.send(&Reply::Progress { id, fraction }),
        |start, s| {
            out.send(&Reply::ShotChunk { id, start, len: s.len() as u32, signals: Blob(signals::encode(s)) });
            Ok(())
        },
    )?;
    let frames = index.len().saturating_sub(job.resume);
    let secs = started.elapsed().as_secs_f64();
    tracing::info!(id, frames, secs, fps = frames as f64 / secs.max(1e-3), "shots measured");
    Ok(())
}

fn run_faces(
    tools: &Tools,
    rt: &Runtime,
    models: &mut Models,
    job: FacesJob,
    cancel: &CancelToken,
    out: &Out,
) -> Result<(), AnalysisError> {
    if job.anime {
        return run_anime_faces(tools, rt, models, job, cancel, out);
    }
    if job.fast {
        return run_fast_faces(tools, rt, models, job, cancel, out);
    }
    let params = match (job.animated, rt.preferred_device()) {
        (false, _) => FaceParams::default(),
        // Searching more sizes takes four times the detector's time: on the
        // CPU, 3D animation is searched at the detection size alone.
        (true, Device::Cpu) => FaceParams { search: Search::Once, ..FaceParams::animated() },
        (true, _) => FaceParams::animated(),
    };
    let (size, _) = frame_sizes(&job.info, &params);
    let known = match &job.known {
        Some(b) => Some(
            tracks::decode_known(&b.0)
                .ok_or_else(|| AnalysisError::Internal("unreadable faces found before".into()))?,
        ),
        None => None,
    };
    if known.is_none() {
        models.load_faces(rt, &detect_looks(&job.info, &params))?;
    }
    if job.bodies {
        models.load_people(rt, size)?;
    }
    let Models { scrfd, arcface, persons, person_reid, .. } = models;
    let device = match known {
        None => scrfd.first().map(Scrfd::device),
        Some(_) => persons.as_ref().map(PpDet::device),
    };
    if known.is_none() && scrfd.iter().any(|d| Some(d.device()) != device) {
        let devices: Vec<_> = scrfd.iter().map(|d| (d.frame_size(), d.device())).collect();
        tracing::warn!(?devices, "face detectors run on different devices");
    }
    let mut used: Vec<(&str, Device)> = Vec::new();
    if known.is_none() {
        used.extend(scrfd.iter().map(|d| (as_models::ids::SCRFD, d.device())));
        used.extend(arcface.as_ref().map(|e| (as_models::ids::ARCFACE, e.device())));
    }
    if job.bodies {
        used.extend(persons.as_ref().map(|d| (as_models::ids::PERSONS, d.device())));
        used.extend(person_reid.as_ref().map(|r| (as_models::ids::PERSON_REID, r.device())));
    }
    send_devices(out, job.id, rt, &used);
    let faces = match &known {
        Some(k) => Faces::Known(k),
        None => Faces::Find { detectors: scrfd, embedder: arcface.as_mut().expect("loaded") },
    };
    let bodies = job.bodies.then(|| BodyModels {
        detector: persons.as_mut().expect("loaded"),
        reid: person_reid.as_mut().expect("loaded"),
        params: PeopleParams::default(),
    });
    let index = FrameIndex::from_bytes(&job.index.0).map_err(|e| AnalysisError::Internal(e.to_string()))?;
    let shots: Vec<FrameRange> = job.shots.iter().map(|&(s, e)| FrameRange::new(s, e)).collect();
    let started = Instant::now();
    let id = job.id;
    let what = if job.known.is_some() { "people" } else { "faces" };
    tracing::info!(
        id,
        file = ?job.path.file_name().unwrap_or_default(),
        resume = job.resume,
        people = job.bodies,
        animated = job.animated,
        ?device,
        "finding {what}"
    );
    let (mut found, mut people) = (0usize, 0usize);
    face_tracks(
        tools,
        Source { path: Path::new(&job.path), info: &job.info, index: &index },
        faces,
        bodies,
        &shots,
        job.resume,
        job.chunk,
        &params,
        cancel,
        |fraction| out.send(&Reply::Progress { id, fraction }),
        |c| {
            found += c.tracks.len();
            people += c.bodies.len();
            let sprites = c
                .sprites
                .iter()
                .map(|s| Sprite { shot: s.shot, end: s.end, cells: s.cells, jpeg: Blob(s.jpeg.clone()) })
                .collect();
            // Nothing when no one was looked for; a list (maybe empty) when they were.
            let found_people = if job.bodies { bodies::encode(c.bodies) } else { Vec::new() };
            out.send(&Reply::FaceChunk {
                id,
                start: c.start,
                end: c.end,
                tracks: Blob(tracks::encode(c.tracks)),
                sprites,
                bodies: Blob(found_people),
            });
            Ok(())
        },
    )?;
    let frames = index.len().saturating_sub(job.resume);
    let secs = started.elapsed().as_secs_f64();
    tracing::info!(id, frames, tracks = found, people, secs, fps = frames as f64 / secs.max(1e-3), "{what} found");
    Ok(())
}

/// A faces job with Fast Scan: faces only, found as `as_analysis::fast` does.
fn run_fast_faces(
    tools: &Tools,
    rt: &Runtime,
    models: &mut Models,
    job: FacesJob,
    cancel: &CancelToken,
    out: &Out,
) -> Result<(), AnalysisError> {
    if job.known.is_some() || job.bodies {
        return Err(AnalysisError::Internal("Fast Scan looks for faces alone".into()));
    }
    let params = FastParams::default();
    models.load_fast(rt, fast::frame_size(&job.info, &params))?;
    let (detector, embedder) = (models.yunet.as_mut().expect("loaded"), models.sface.as_mut().expect("loaded"));
    let device = detector.device();
    if embedder.device() != device {
        tracing::warn!(detector = ?device, embedder = ?embedder.device(), "Fast Scan's models run on different devices");
    }
    send_devices(out, job.id, rt, &[(as_models::ids::YUNET, device), (as_models::ids::SFACE, embedder.device())]);
    let index = FrameIndex::from_bytes(&job.index.0).map_err(|e| AnalysisError::Internal(e.to_string()))?;
    let shots: Vec<FrameRange> = job.shots.iter().map(|&(s, e)| FrameRange::new(s, e)).collect();
    let started = Instant::now();
    let (id, file) = (job.id, job.path.file_name().unwrap_or_default());
    tracing::info!(id, ?file, resume = job.resume, ?device, "finding faces (Fast Scan)");
    let mut found = 0usize;
    fast_face_tracks(
        tools,
        Source { path: Path::new(&job.path), info: &job.info, index: &index },
        FastModels { detector, embedder },
        &shots,
        job.resume,
        job.chunk,
        &params,
        cancel,
        |fraction| out.send(&Reply::Progress { id, fraction }),
        |c| {
            found += c.tracks.len();
            let sprites = c
                .sprites
                .iter()
                .map(|s| Sprite { shot: s.shot, end: s.end, cells: s.cells, jpeg: Blob(s.jpeg.clone()) })
                .collect();
            out.send(&Reply::FaceChunk {
                id,
                start: c.start,
                end: c.end,
                tracks: Blob(tracks::encode(c.tracks)),
                sprites,
                bodies: Blob(Vec::new()),
            });
            Ok(())
        },
    )?;
    let frames = index.len().saturating_sub(job.resume);
    let secs = started.elapsed().as_secs_f64();
    tracing::info!(id, frames, tracks = found, secs, fps = frames as f64 / secs.max(1e-3), "faces found (Fast Scan)");
    Ok(())
}

/// A faces job on anime footage: characters only, found as
/// `as_analysis::anime` does (heads, implied heads of person detections,
/// CCIP) — with `fast`, in the frames Fast Scan picks. Nobody is looked for
/// by their body and no faces found before are carried in — the pass
/// stands alone like Fast Scan.
fn run_anime_faces(
    tools: &Tools,
    rt: &Runtime,
    models: &mut Models,
    job: FacesJob,
    cancel: &CancelToken,
    out: &Out,
) -> Result<(), AnalysisError> {
    if job.known.is_some() || job.bodies || job.animated {
        return Err(AnalysisError::Internal("the anime pass looks for characters alone".into()));
    }
    let params = anime::Params::anime();
    let fast_params = job.fast.then(FastParams::anime);
    let (size, batch) = match &fast_params {
        Some(p) => (fast::frame_size(&job.info, p), FAST_DETECT_BATCH),
        None => (anime::frame_size(&job.info, &params), DETECT_BATCH),
    };
    models.load_anime(rt, size, batch)?;
    let (heads, persons, ccip) = (
        models.anime_heads.as_mut().expect("loaded"),
        models.anime_persons.as_mut().expect("loaded"),
        models.anime_ccip.as_mut().expect("loaded"),
    );
    let device = heads.device();
    if persons.device() != device || ccip.device() != device {
        tracing::warn!(
            heads = ?device,
            persons = ?persons.device(),
            ccip = ?ccip.device(),
            "the anime models run on different devices"
        );
    }
    send_devices(
        out,
        job.id,
        rt,
        &[
            (as_models::ids::ANIME_HEAD, device),
            (as_models::ids::ANIME_PERSON, persons.device()),
            (as_models::ids::ANIME_CCIP, ccip.device()),
        ],
    );
    let index = FrameIndex::from_bytes(&job.index.0).map_err(|e| AnalysisError::Internal(e.to_string()))?;
    let shots: Vec<FrameRange> = job.shots.iter().map(|&(s, e)| FrameRange::new(s, e)).collect();
    let started = Instant::now();
    let (id, file) = (job.id, job.path.file_name().unwrap_or_default());
    let fast = job.fast;
    tracing::info!(id, ?file, resume = job.resume, ?device, fast, "finding characters (anime)");
    let mut found = 0usize;
    let src = Source { path: Path::new(&job.path), info: &job.info, index: &index };
    let anime = anime::Anime { heads, persons: Some(persons), ccip };
    let progress = |fraction| out.send(&Reply::Progress { id, fraction });
    let on_chunk = |c: FaceChunk| {
        found += c.tracks.len();
        let sprites = c
            .sprites
            .iter()
            .map(|s| Sprite { shot: s.shot, end: s.end, cells: s.cells, jpeg: Blob(s.jpeg.clone()) })
            .collect();
        out.send(&Reply::FaceChunk {
            id,
            start: c.start,
            end: c.end,
            tracks: Blob(tracks::encode(c.tracks)),
            sprites,
            bodies: Blob(Vec::new()),
        });
        Ok(())
    };
    match &fast_params {
        Some(p) => fast_face_tracks(
            tools,
            src,
            anime::FastAnime { anime, params: &params },
            &shots,
            job.resume,
            job.chunk,
            p,
            cancel,
            progress,
            on_chunk,
        )?,
        None => {
            anime::face_tracks(tools, src, anime, &shots, job.resume, job.chunk, &params, cancel, progress, on_chunk)?
        }
    }
    let frames = index.len().saturating_sub(job.resume);
    let secs = started.elapsed().as_secs_f64();
    tracing::info!(
        id,
        frames,
        tracks = found,
        secs,
        fps = frames as f64 / secs.max(1e-3),
        fast,
        "characters found (anime)"
    );
    Ok(())
}

/// Embed a still image with the anime embedder: no detector runs on a
/// portrait — the whole picture, centred in a square of its longer side
/// (`ccip::crop` pads the rest with black), is embedded like a head's
/// context crop.
fn run_embed(
    tools: &Tools,
    rt: &Runtime,
    models: &mut Models,
    job: EmbedJob,
    cancel: &CancelToken,
    out: &Out,
) -> Result<(), AnalysisError> {
    models.load_ccip(rt)?;
    let ccip = models.anime_ccip.as_mut().expect("loaded");
    send_devices(out, job.id, rt, &[(as_models::ids::ANIME_CCIP, ccip.device())]);
    let (w, h, rgb) = frames::decode_still(tools, &job.path, cancel)?;
    let (w, h) = (w as usize, h as usize);
    let side = w.max(h) as f32;
    let square = as_vision::ccip::crop(&rgb, w, h, (w as f32 - side) / 2.0, (h as f32 - side) / 2.0, side);
    let e = ccip
        .embed(&[square.as_slice()])?
        .into_iter()
        .next()
        .ok_or_else(|| AnalysisError::Internal("the embedder gave no vector".into()))?;
    out.send(&Reply::Embedding { id: job.id, embedding: e.v });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_that_panics_fails_with_its_message() {
        assert_eq!(guarded(|| panic!("index out of bounds")).unwrap_err(), "index out of bounds");
        let n = 3;
        assert_eq!(guarded(|| panic!("frame {n} is missing")).unwrap_err(), "frame 3 is missing");
        assert!(guarded(|| Ok(())).unwrap().is_ok());
        assert!(guarded(|| Err(AnalysisError::Internal("no".into()))).unwrap().is_err());
    }
}

//! Plugin runs, with the stand-in plugin (`src/bin/asp_test_plugin.rs`):
//! what reaches it, what it says, and how a run ends when it misbehaves.
//! Each run works in a folder of its own.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use std::{env, fs};

use as_core::{AppPaths, CancelToken};
use as_db::LibraryDb;
use as_plugin::log::MAX_LOG_BYTES;
use as_plugin::protocol::{ClipContext, ExportContext, ProjectContext};
use as_plugin::runtime::{self, CANCEL_GRACE, EXIT_GRACE, MAX_LINE_BYTES, RunSpec, TERM_GRACE};
use as_plugin::{
    ActionContext, ActionKind, AppContext, Keys, Launch, PluginContext, PluginError, PluginId, PluginResult,
    RunMessage, SettingValue, Store, ToolPaths, Trigger, current_platform,
};
use semver::Version;
use serde_json::{Value, json};
use tempfile::TempDir;

const NAME: &str = "Test Plugin";
const ID: &str = "dev.autoscene.test-plugin";

/// A run of the stand-in plugin in `mode`, in a folder of its own.
struct Plugin {
    dir: TempDir,
    mode: &'static str,
    message: RunMessage,
}

impl Plugin {
    fn new(mode: &'static str) -> Self {
        Self { dir: tempfile::tempdir().unwrap(), mode, message: message() }
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn log(&self) -> PathBuf {
        self.dir.path().join("logs").join(format!("{ID}.log"))
    }

    fn log_text(&self) -> String {
        fs::read_to_string(self.log()).unwrap()
    }

    fn spec(&self) -> RunSpec {
        let exe = Path::new(env!("CARGO_BIN_EXE_asp_test_plugin"));
        RunSpec {
            name: NAME.into(),
            dir: exe.parent().unwrap().to_path_buf(),
            launch: Launch::Exe {
                program: exe.file_name().unwrap().to_str().unwrap().into(),
                args: vec![self.mode.into()],
            },
            data_dir: self.data(),
            log_path: self.log(),
            ffmpeg: self.dir.path().join("tools").join("ffmpeg"),
            ffprobe: self.dir.path().join("tools").join("ffprobe"),
            message: self.message.clone(),
        }
    }

    fn run(&self) -> PluginResult<Option<String>> {
        runtime::run(self.spec(), &CancelToken::new(), &|_, _| {})
    }

    /// Runs it, cancelled once its grandchild beats: the result, and how
    /// long the run took from the cancel on.
    fn run_cancelled(&self) -> (PluginResult<Option<String>>, Duration) {
        let beat = self.data().join("beat");
        self.run_cancelled_once(|| fs::metadata(&beat).is_ok_and(|m| m.len() > 0))
    }

    /// Runs it, cancelled once `ready`: the result, and how long the run
    /// took from the cancel on.
    fn run_cancelled_once(&self, ready: impl Fn() -> bool) -> (PluginResult<Option<String>>, Duration) {
        let cancel = CancelToken::new();
        thread::scope(|s| {
            let run = s.spawn(|| runtime::run(self.spec(), &cancel, &|_, _| {}));
            wait_for(ready);
            let asked = Instant::now();
            cancel.cancel();
            (run.join().unwrap(), asked.elapsed())
        })
    }
}

fn message() -> RunMessage {
    RunMessage {
        protocol: 1,
        run_id: "run-1".into(),
        plugin: PluginContext { id: PluginId::parse(ID).unwrap(), version: "1.2.0".into(), data_dir: "/data".into() },
        app: AppContext { version: "1.0.3-BETA".into(), platform: "linux-x86_64".into() },
        tools: ToolPaths {
            ffmpeg: "/tools/ffmpeg".into(),
            ffprobe: "/tools/ffprobe".into(),
            ffmpeg_version: "9.0".into(),
        },
        settings: BTreeMap::new(),
        action: ActionContext { kind: ActionKind::ExportAction, id: "post".into(), trigger: Trigger::Menu },
        project: None,
        export: None,
    }
}

fn wait_for(done: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < until, "timed out");
        thread::sleep(Duration::from_millis(20));
    }
}

/// Whether the heartbeat in `file` stopped: it grows by a byte every 50 ms
/// while its process lives.
fn stopped(file: &Path) -> bool {
    // A process the system was told to end may take a moment.
    thread::sleep(Duration::from_millis(300));
    let before = fs::metadata(file).unwrap().len();
    thread::sleep(Duration::from_millis(600));
    fs::metadata(file).unwrap().len() == before
}

#[test]
fn a_result_line_gives_the_message() {
    let p = Plugin::new("result");
    let stages = Mutex::new(Vec::new());
    let result = runtime::run(p.spec(), &CancelToken::new(), &|stage, fraction| {
        stages.lock().unwrap().push((stage.to_owned(), fraction));
    });
    assert_eq!(result.unwrap().as_deref(), Some("Posted 12 clips."));
    assert_eq!(stages.into_inner().unwrap(), [("Posting".to_owned(), Some(0.5))]);
    let log = p.log_text();
    assert!(log.starts_with("--- ") && log.lines().next().unwrap().ends_with("Z run run-1 post"), "{log}");
    assert!(log.contains("[result] Posted 12 clips.\n"), "{log}");
    // A plugin that ends well without a word has nothing to say.
    assert_eq!(Plugin::new("quiet").run().unwrap(), None);
}

#[test]
fn the_run_message_is_one_line_with_the_full_context() {
    let mut p = Plugin::new("echo");
    p.message.settings = BTreeMap::from([
        ("count".into(), SettingValue::Number(3.0)),
        ("loud".into(), SettingValue::Bool(true)),
        ("webhook".into(), SettingValue::Text("https://discord.com/api/webhooks/1/x".into())),
    ]);
    p.message.action.trigger = Trigger::AfterExport;
    p.message.project = Some(ProjectContext { id: "p1".into(), title: "Breaking Bad".into(), folder: "/bb".into() });
    p.message.export = Some(ExportContext {
        id: "e1".into(),
        label: Some("Jesse".into()),
        preset: "prores".into(),
        out_dir: "/out".into(),
        created_at: 1_762_000_000_000,
        merged_file: None,
        clips: vec![ClipContext {
            name: "Jesse 001.mov".into(),
            path: Some("/out/Jesse 001.mov".into()),
            status: "done".into(),
            media_id: "m1".into(),
            source_path: "/media/S01E01.mkv".into(),
            episode: "S01E01".into(),
            start: 120,
            end: 480,
            fps: Some("24000/1001".into()),
        }],
    });
    // The protocol, the data folder and FFmpeg's paths are the run's own,
    // whatever the message held ("/data", "/tools/ffmpeg").
    p.message.protocol = 7;
    assert_eq!(p.run().unwrap().as_deref(), Some("Echoed."));
    let line = fs::read_to_string(p.data().join("run.json")).unwrap();
    let json = |path: PathBuf| serde_json::to_string(&path).unwrap();
    let (data, ffmpeg, ffprobe) = (json(p.data()), json(p.spec().ffmpeg), json(p.spec().ffprobe));
    assert_eq!(
        line,
        [
            r#"{"type":"run","protocol":1,"runId":"run-1","#,
            &format!(r#""plugin":{{"id":"dev.autoscene.test-plugin","version":"1.2.0","dataDir":{data}}},"#),
            r#""app":{"version":"1.0.3-BETA","platform":"linux-x86_64"},"#,
            &format!(r#""tools":{{"ffmpeg":{ffmpeg},"ffprobe":{ffprobe},"ffmpegVersion":"9.0"}},"#),
            r#""settings":{"count":3.0,"loud":true,"webhook":"https://discord.com/api/webhooks/1/x"},"#,
            r#""action":{"kind":"exportAction","id":"post","trigger":"afterExport"},"#,
            r#""project":{"id":"p1","title":"Breaking Bad","folder":"/bb"},"#,
            r#""export":{"id":"e1","label":"Jesse","preset":"prores","outDir":"/out","createdAt":1762000000000,"#,
            r#""mergedFile":null,"clips":[{"name":"Jesse 001.mov","path":"/out/Jesse 001.mov","status":"done","#,
            r#""mediaId":"m1","sourcePath":"/media/S01E01.mkv","episode":"S01E01","start":120,"end":480,"#,
            r#""fps":"24000/1001"}]}}"#,
            "\n",
        ]
        .concat()
    );
}

#[test]
fn an_installed_plugin_runs_in_its_own_folders_with_its_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::at(tmp.path().join("data"));
    let db = Arc::new(LibraryDb::open(&paths.library_db()).unwrap());
    let app = Version::parse("1.0.3-BETA").unwrap();
    let store = Store::open(&paths, db, Keys::new(&[]).unwrap(), app).unwrap();
    // The stand-in plugin in a developer's folder, loaded from there.
    let work = tmp.path().join("work");
    fs::create_dir(&work).unwrap();
    let exe = Path::new(env!("CARGO_BIN_EXE_asp_test_plugin"));
    let program = exe.file_name().unwrap().to_str().unwrap();
    fs::copy(exe, work.join(program)).unwrap();
    let manifest = json!({
        "manifestVersion": 1,
        "id": ID,
        "version": "1.2.0",
        "name": NAME,
        "description": "Stands in for a plugin.",
        "author": {"name": "AutoScene"},
        "engines": {"autoscene": ">=1.0.0", "protocol": 1},
        "runtime": {"type": "process", "command": {"*": [program, "echo"]}},
        "contributes": {
            "exportActions": [{"id": "post", "title": "Post"}],
            "settings": [
                {"key": "greeting", "type": "string", "title": "Greeting", "default": "Hi"},
                {"key": "count", "type": "number", "title": "Count", "default": 3}
            ]
        }
    });
    fs::write(work.join("plugin.json"), manifest.to_string()).unwrap();
    let p = store.commit(store.stage_folder(&work).unwrap(), true, true).unwrap();
    let id = PluginId::parse(ID).unwrap();
    store.set_values(&id, BTreeMap::from([("greeting".into(), SettingValue::Text("Hey".into()))])).unwrap();

    let action = || ActionContext { kind: ActionKind::ExportAction, id: "post".into(), trigger: Trigger::AfterExport };
    let tools = tmp.path().join("tools");
    let (ffmpeg, ffprobe) = (tools.join("ffmpeg"), tools.join("ffprobe"));
    let spec = store.run_spec(&p, action(), &ffmpeg, &ffprobe, "9.0.2").unwrap();
    let (data, log) = (paths.plugin_data().join(ID), paths.plugin_logs().join(format!("{ID}.log")));
    assert_eq!((&spec.dir, &spec.data_dir, &spec.log_path), (&work, &data, &log));
    assert_eq!(runtime::run(spec, &CancelToken::new(), &|_, _| {}).unwrap().as_deref(), Some("Echoed."));

    let line: Value = serde_json::from_str(&fs::read_to_string(data.join("run.json")).unwrap()).unwrap();
    assert_eq!(line["plugin"], json!({"id": ID, "version": "1.2.0", "dataDir": data}));
    assert_eq!(line["app"], json!({"version": "1.0.3-BETA", "platform": current_platform()}));
    assert_eq!(line["tools"], json!({"ffmpeg": ffmpeg, "ffprobe": ffprobe, "ffmpegVersion": "9.0.2"}));
    // The person's value, and the default of the one left alone.
    assert_eq!(line["settings"], json!({"count": 3.0, "greeting": "Hey"}));
    assert_eq!(line["action"], json!({"kind": "exportAction", "id": "post", "trigger": "afterExport"}));
    let run_id = line["runId"].as_str().unwrap();
    let header = fs::read_to_string(&log).unwrap().lines().next().unwrap().to_owned();
    assert!(header.ends_with(&format!(" run {run_id} post")), "{header}");
    // Each run has an id of its own.
    let again = store.run_spec(&p, action(), &ffmpeg, &ffprobe, "9.0.2").unwrap();
    assert_ne!(again.message.run_id, run_id);
}

#[test]
fn only_allowlisted_variables_reach_the_plugin() {
    // SAFETY: the tests here read the environment only through std, which
    // holds a lock while it does.
    unsafe {
        env::set_var("AUTOSCENE_TEST_SECRET", "hunter2");
        env::set_var("AUTOSCENE_DATA_DIR", "/elsewhere");
        env::set_var("AUTOSCENE_FFMPEG_DIR", "/elsewhere");
        env::set_var("RUST_LOG", "trace");
        env::set_var("CUDA_PATH_V13_0", "/cuda");
    }
    let p = Plugin::new("env");
    assert_eq!(p.run().unwrap(), None);
    let dump: Value = serde_json::from_str(&fs::read_to_string(p.data().join("env.json")).unwrap()).unwrap();
    let vars = dump["vars"].as_object().unwrap();
    let ours = [
        "AUTOSCENE_FFMPEG",
        "AUTOSCENE_FFPROBE",
        "AUTOSCENE_PLUGIN_DATA",
        "AUTOSCENE_PLUGIN_ID",
        "AUTOSCENE_PROTOCOL",
        "PYTHONUNBUFFERED",
        "PYTHONUTF8",
        "PYTHONIOENCODING",
    ];
    let passed = env::vars_os().map(|(k, _)| k).filter(|k| runtime::allowed_env(k));
    let expected: BTreeSet<OsString> = passed.chain(ours.iter().map(OsString::from)).collect();
    let got: BTreeSet<OsString> = vars.keys().map(OsString::from).collect();
    assert_eq!(got, expected);
    assert!(got.contains(&OsString::from("CUDA_PATH_V13_0")));
    for name in ["AUTOSCENE_TEST_SECRET", "AUTOSCENE_DATA_DIR", "AUTOSCENE_FFMPEG_DIR", "RUST_LOG"] {
        assert!(!vars.contains_key(name), "{name}");
    }
    let data = p.data();
    let text = |name: &str| vars[name].as_str().unwrap().to_owned();
    assert_eq!(text("AUTOSCENE_PLUGIN_ID"), ID);
    assert_eq!(text("AUTOSCENE_PROTOCOL"), "1");
    assert_eq!(text("PYTHONUTF8"), "1");
    assert_eq!(PathBuf::from(text("AUTOSCENE_PLUGIN_DATA")), data);
    assert_eq!(PathBuf::from(text("AUTOSCENE_FFMPEG")), p.dir.path().join("tools").join("ffmpeg"));
    // It runs in its data folder.
    let cwd = PathBuf::from(dump["cwd"].as_str().unwrap());
    assert_eq!(cwd.canonicalize().unwrap(), data.canonicalize().unwrap());
}

#[cfg(windows)]
#[test]
fn windows_plugins_get_systemroot() {
    let p = Plugin::new("env");
    p.run().unwrap();
    let dump: Value = serde_json::from_str(&fs::read_to_string(p.data().join("env.json")).unwrap()).unwrap();
    let vars = dump["vars"].as_object().unwrap();
    let get = |name: &str| vars.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str().unwrap());
    let root = env::var("SystemRoot").unwrap();
    assert_eq!(get("SystemRoot"), Some(root.as_str()));
    for name in ["windir", "PATHEXT", "TEMP", "LOCALAPPDATA", "PROCESSOR_ARCHITECTURE"] {
        assert_eq!(get(name).map(str::to_owned), env::var(name).ok(), "{name}");
    }
}

#[test]
fn a_plugin_that_floods_its_pipes_still_finishes() {
    let mut p = Plugin::new("flood");
    // A run line far past any pipe's buffer, which the plugin reads only
    // after its flood: written on the run's own thread, it would hang both.
    p.message.settings = BTreeMap::from([("big".into(), SettingValue::Text("s".repeat(1 << 20)))]);
    let (tx, rx) = mpsc::channel();
    let spec = p.spec();
    thread::spawn(move || tx.send(runtime::run(spec, &CancelToken::new(), &|_, _| {})));
    let result = rx.recv_timeout(Duration::from_secs(20)).expect("the run hung");
    assert_eq!(result.unwrap().as_deref(), Some("Flooded."));
    let log = p.log_text();
    assert!(log.len() as u64 <= MAX_LOG_BYTES + 200, "{}", log.len());
    assert_eq!(log.matches("[host] more output dropped\n").count(), 1);
    assert!(log.contains(&format!("[stderr] {}\n", "e".repeat(1023))));
    // The host's word outlasts the flood.
    let end: Vec<_> = log.lines().rev().take(3).collect();
    assert_eq!(end[2], "[host] more output dropped");
    assert_eq!(end[1], "[result] Flooded.");
    assert!(end[0].starts_with("[host] exit ") && end[0].ends_with(": 0"), "{}", end[0]);
}

#[test]
fn non_json_lines_go_to_the_log() {
    let p = Plugin::new("chatty");
    // The first result counts.
    assert_eq!(p.run().unwrap().as_deref(), Some("Said hello."));
    let log = p.log_text();
    for line in [
        "[stdout] hello from a plugin\n",
        "[stdout] inner\n  return\n",
        "[info] two\n  [host] exit code: 0\n  --- 2026-01-01T00:00:00Z run forged post\n",
        "[stdout] {\"type\":\"future\",\"x\":1}\n",
        "[stdout] {\"type\":\"progress\",\"fraction\":\"half\"}\n",
        "[warn] careful\n",
        "[info] a level from later\n",
        "[result] Said hello.\n",
        "[result] Said it twice.\n",
    ] {
        assert!(log.contains(line), "{line}: {log}");
    }
    // Only the host starts a line with `---` or `[host]`.
    assert_eq!(log.lines().filter(|l| l.starts_with("---")).count(), 1, "{log}");
    assert_eq!(log.lines().filter(|l| l.starts_with("[host]")).count(), 1, "{log}");
}

#[test]
fn a_cut_line_is_no_message() {
    let p = Plugin::new("padded");
    // Its first 64 KB read as a result, but its end is gone.
    assert_eq!(p.run().unwrap(), None);
    let log = p.log_text();
    let line = log.lines().find(|l| l.starts_with("[stdout] ")).unwrap();
    assert!(line.starts_with(r#"[stdout] {"type":"result","message":"x"}  "#) && line.ends_with(" [cut]"), "{log}");
}

#[test]
fn an_error_line_fails_the_run_with_its_message() {
    let p = Plugin::new("error");
    match p.run() {
        Err(PluginError::Failed { message, detail }) => {
            assert_eq!(message, "No webhook is set. Set one in Settings.");
            // Cleaned for the app's log, and cut.
            let detail = detail.unwrap();
            assert!(detail.starts_with("KeyError: 'webhook' [2m at main.py:12 frame frame "), "{detail}");
            assert_eq!(detail.chars().count(), 2000);
            assert!(detail.ends_with('…') && !detail.chars().any(char::is_control), "{detail}");
        }
        other => panic!("{other:?}"),
    }
    // The plugin's log has it as it came, a line for each of its lines.
    let log = p.log_text();
    let failed = "[failed] No webhook is set.\n  Set one in Settings. (KeyError: 'webhook'\n  \u{1b}[2m  at main.py:12\n  frame ";
    assert!(log.contains(failed), "{log}");
    assert!(log.contains(&format!("\n  {})\n", "frame ".repeat(400))), "{log}");
}

#[test]
fn a_crash_says_so_and_keeps_the_stderr_tail_in_the_log() {
    let p = Plugin::new("crash");
    let e = p.run().unwrap_err();
    assert_eq!(e.to_string(), format!("{NAME} stopped unexpectedly."));
    let PluginError::Crashed { name, detail } = e else { panic!("{e:?}") };
    assert_eq!(name, NAME);
    let (status, tail) = detail.split_once("; ").unwrap();
    assert!(status.contains('3'), "{detail}");
    // The last 20 lines, each cleaned for the app's log and cut.
    let mut lines: Vec<String> = (11..30).map(|i| format!("problem {i}")).collect();
    lines.push(format!("[31mred [0m over {}…", "w".repeat(482)));
    assert_eq!(tail, lines.join("\n"));
    assert_eq!(lines[19].chars().count(), 500);
    let log = p.log_text();
    for i in 0..30 {
        assert!(log.contains(&format!("[stderr] problem {i}\n")), "{log}");
    }
    assert!(log.contains(&format!("[stderr] \u{1b}[31mred\u{1b}[0m\n  over {}\n", "w".repeat(600))), "{log}");
    assert!(log.contains(&format!("[host] {status}\n")), "{log}");
}

#[test]
fn cancel_asks_first_then_kills_the_tree() {
    let p = Plugin::new("cancel");
    let (result, took) = p.run_cancelled();
    assert!(matches!(result, Err(PluginError::Cancelled)), "{result:?}");
    assert!(took < CANCEL_GRACE, "{took:?}");
    assert_eq!(fs::read_to_string(p.data().join("cancel-seen")).unwrap(), "{\"type\":\"cancel\"}\n");
    // The plugin ended on its own; what it left running did not.
    assert!(stopped(&p.data().join("beat")));
    assert!(p.log_text().contains("[host] asked to cancel\n"));
}

#[test]
fn a_plugin_ignoring_cancel_and_its_grandchild_are_killed() {
    let p = Plugin::new("ignore");
    let (result, took) = p.run_cancelled();
    assert!(matches!(result, Err(PluginError::Cancelled)), "{result:?}");
    assert!(took >= CANCEL_GRACE, "{took:?}");
    assert!(took < CANCEL_GRACE + TERM_GRACE + Duration::from_secs(5), "{took:?}");
    assert!(stopped(&p.data().join("beat")));
    assert!(p.log_text().contains("[host] still running after cancel; ended\n"));
}

#[test]
fn a_plugin_that_hangs_after_its_result_is_ended() {
    let p = Plugin::new("linger");
    let started = Instant::now();
    assert_eq!(p.run().unwrap().as_deref(), Some("Done."));
    let took = started.elapsed();
    assert!(took >= EXIT_GRACE, "{took:?}");
    assert!(took < EXIT_GRACE + TERM_GRACE + Duration::from_secs(5), "{took:?}");
    assert!(p.log_text().contains("[host] still running after its result; ended\n"));
}

#[test]
fn its_stdin_closes_once_its_result_is_in() {
    let p = Plugin::new("eof");
    let started = Instant::now();
    assert_eq!(p.run().unwrap().as_deref(), Some("Closed."));
    // It saw its stdin end, with nothing after the run line, and exited.
    assert_eq!(fs::read_to_string(p.data().join("stdin-closed")).unwrap(), "");
    assert!(started.elapsed() < EXIT_GRACE / 2, "{:?}", started.elapsed());
}

#[test]
fn a_cancel_after_the_result_keeps_it_and_ends_the_plugin_at_once() {
    let p = Plugin::new("linger");
    // The run logs the line that counts before it looks at the cancel again.
    let (result, took) =
        p.run_cancelled_once(|| fs::read_to_string(p.log()).is_ok_and(|log| log.contains("[result] Done.\n")));
    assert_eq!(result.unwrap().as_deref(), Some("Done."));
    assert!(took < EXIT_GRACE / 2, "{took:?}");
    let log = p.log_text();
    assert!(log.contains("[host] cancelled after its result; ended\n"), "{log}");
    // A plugin that said its last word isn't asked to stop.
    assert!(!log.contains("asked to cancel"), "{log}");
}

#[test]
fn a_run_cancelled_before_it_starts_starts_nothing() {
    let p = Plugin::new("result");
    let cancel = CancelToken::new();
    cancel.cancel();
    let result = runtime::run(p.spec(), &cancel, &|_, _| {});
    assert!(matches!(result, Err(PluginError::Cancelled)), "{result:?}");
    assert!(!p.data().exists() && !p.log().exists());
}

#[test]
fn an_endless_line_is_cut_at_the_limit() {
    let p = Plugin::new("long");
    assert_eq!(p.run().unwrap().as_deref(), Some("Long."));
    let log = p.log_text();
    assert!(log.contains(&format!("\n[stdout] {} [cut]\n", "x".repeat(MAX_LINE_BYTES))), "{}", log.len());
    assert!(log.contains(&format!("\n[stderr] {} [cut]\n", "z".repeat(MAX_LINE_BYTES))), "{}", log.len());
}

#[test]
fn stage_text_is_cleaned_and_capped() {
    let p = Plugin::new("stages");
    let stages = Mutex::new(Vec::new());
    let result = runtime::run(p.spec(), &CancelToken::new(), &|stage, fraction| {
        stages.lock().unwrap().push((stage.to_owned(), fraction));
    });
    let message = result.unwrap().unwrap();
    assert_eq!(message.chars().count(), 300);
    assert!(message.starts_with("word word ") && message.ends_with('…') && !message.contains("  "), "{message}");
    let long = format!("{}…", "a".repeat(79));
    assert_eq!(
        stages.into_inner().unwrap(),
        [
            ("Posting clips to Discord".to_owned(), Some(0.5)),
            (long, Some(1.0)),
            (NAME.to_owned(), Some(0.0)),
            (NAME.to_owned(), None),
        ]
    );
}

#[test]
fn a_missing_program_says_so() {
    let p = Plugin::new("result");
    let mut spec = p.spec();
    spec.launch = Launch::Exe { program: "bin/gone.exe".into(), args: Vec::new() };
    let e = runtime::run(spec, &CancelToken::new(), &|_, _| {}).unwrap_err();
    assert_eq!(e.to_string(), format!("{NAME} can't start: one of its files is missing."));
    // Nothing started, so nothing was made.
    assert!(!p.data().exists() && !p.log().exists());
}

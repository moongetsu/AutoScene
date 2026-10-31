//! ONNX Runtime for the analysis engine.
//!
//! The runtime is the copy the app ships (`cargo xtask fetch-sidecars` →
//! `onnxruntime/`), loaded by absolute path: Windows has an older
//! `onnxruntime.dll` and `DirectML.dll` in System32 that must never be picked
//! up. Sessions run on the GPU — DirectML on Windows (any DX12 GPU), CoreML on
//! macOS — and fall back to the CPU when the GPU provider can't take a model.
//!
//! `load-dynamic` binds one runtime per process, which is one reason the
//! engine is its own process: switching runtimes means restarting it.

pub mod onnx;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use ort::inputs;
pub use ort::session::Session;
pub use ort::value::{Tensor, TensorRef};
use thiserror::Error;

pub type InferResult<T> = Result<T, InferError>;

#[derive(Debug, Error)]
pub enum InferError {
    #[error("AutoScene's AI runtime is missing. Reinstall AutoScene.")]
    Missing(PathBuf),
    #[error("AutoScene's AI runtime couldn't start. Details are in the log.")]
    Runtime(String),
    #[error("An AI model couldn't be loaded. Details are in the log.")]
    Model(String),
    #[error("The AI model failed on this frame. Details are in the log.")]
    Run(String),
}

impl InferError {
    pub fn detail(&self) -> String {
        match self {
            Self::Missing(p) => format!("onnxruntime not found at {}", p.display()),
            Self::Runtime(e) => format!("onnxruntime init: {e}"),
            Self::Model(e) => format!("model load: {e}"),
            Self::Run(e) => format!("inference: {e}"),
        }
    }
}

impl From<ort::Error> for InferError {
    fn from(e: ort::Error) -> Self {
        Self::Run(e.to_string())
    }
}

/// Where a session computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    DirectMl,
    CoreMl,
    Cpu,
}

impl Device {
    pub fn label(self) -> &'static str {
        match self {
            Self::DirectMl => "GPU (DirectML)",
            Self::CoreMl => "Apple Neural Engine / GPU (Core ML)",
            Self::Cpu => "CPU",
        }
    }
}

/// File name of the runtime library in the bundle folder.
pub const LIBRARY: &str = if cfg!(windows) {
    "onnxruntime.dll"
} else if cfg!(target_os = "macos") {
    "libonnxruntime.dylib"
} else {
    "libonnxruntime.so"
};

pub struct Runtime {
    env: ort::environment::Environment,
    gpu: bool,
}

static LOADED: OnceLock<PathBuf> = OnceLock::new();

impl Runtime {
    /// Load the runtime from `dir` (once per process; later calls must name
    /// the same folder). `gpu = false` keeps every session on the CPU.
    pub fn load(dir: &Path, gpu: bool) -> InferResult<Self> {
        // LoadLibraryEx with an altered search path needs an absolute path.
        let dir = &std::path::absolute(dir).map_err(|e| InferError::Runtime(format!("{}: {e}", dir.display())))?;
        let lib = dir.join(LIBRARY);
        if !lib.is_file() {
            return Err(InferError::Missing(lib));
        }
        if let Some(prev) = LOADED.get() {
            if prev != dir {
                return Err(InferError::Runtime(format!("already loaded from {}", prev.display())));
            }
        } else {
            #[cfg(windows)]
            preload_windows(dir);
            ort::init_from(&lib).map_err(|e| InferError::Runtime(e.to_string()))?;
            let _ = LOADED.set(dir.to_path_buf());
        }
        let env = ort::init().with_name("autoscene").build().map_err(|e| InferError::Runtime(e.to_string()))?;
        tracing::info!(runtime = %name(&lib), gpu, "onnxruntime loaded");
        Ok(Self { env, gpu })
    }

    /// The same runtime with every session on the CPU (one runtime per
    /// process: there is no second `load`), e.g. to check GPU results.
    pub fn on_cpu(&self) -> Self {
        Self { env: self.env.clone(), gpu: false }
    }

    /// The device sessions will try first.
    pub fn preferred_device(&self) -> Device {
        match self.gpu {
            true if cfg!(windows) => Device::DirectMl,
            true if cfg!(target_os = "macos") => Device::CoreMl,
            _ => Device::Cpu,
        }
    }

    /// The same runtime, but sessions built so CoreML computes in fp32
    /// (`MLProgram`, no low-precision accumulation on the GPU): the anime
    /// models' embeddings bend under fp16 maths. Everywhere else it builds
    /// exactly like this runtime.
    pub fn fp32_sessions(&self) -> Fp32<'_> {
        Fp32 { rt: self }
    }

    /// Load `model`, on the GPU when possible. Returns where it runs.
    pub fn session(&self, model: &Path) -> InferResult<(Session, Device)> {
        self.session_from(model, None, false)
    }

    /// Like [`Runtime::session`], with the shapes the model declares for its
    /// outputs and intermediate values dropped and its input shaped as
    /// `input` says (see [`onnx::reshaped`]): for graphs that compute any
    /// input size or batch but were exported declaring one.
    pub fn session_shaped(&self, model: &Path, input: onnx::InputShape) -> InferResult<(Session, Device)> {
        let bytes = std::fs::read(model).map_err(|e| InferError::Model(format!("{}: {e}", model.display())))?;
        let patched = onnx::reshaped(&bytes, input).ok_or_else(|| {
            InferError::Model(format!("{}: not a readable ONNX model, or not shaped like {input:?}", model.display()))
        })?;
        self.session_from(model, Some(&patched), false)
    }

    /// Like [`Runtime::session_shaped`], for an export that pins its batch
    /// at 1 inside the graph too: its constant Reshape targets copy the
    /// batch instead ([`onnx::free_reshape_batch`]).
    pub fn session_batched(&self, model: &Path, input: onnx::InputShape) -> InferResult<(Session, Device)> {
        let bytes = std::fs::read(model).map_err(|e| InferError::Model(format!("{}: {e}", model.display())))?;
        let unreadable = |what: &str| InferError::Model(format!("{}: {what}", model.display()));
        let (free, targets) = onnx::free_reshape_batch(&bytes)
            .ok_or_else(|| unreadable("not a readable ONNX model, or a Reshape that takes 0 literally"))?;
        let patched = onnx::reshaped(&free, input).ok_or_else(|| unreadable(&format!("not shaped like {input:?}")))?;
        tracing::debug!(model = %model.display(), targets, "Reshapes copy the batch");
        self.session_from(model, Some(&patched), false)
    }

    fn session_from(&self, model: &Path, bytes: Option<&[u8]>, fp32: bool) -> InferResult<(Session, Device)> {
        let want = self.preferred_device();
        if want != Device::Cpu {
            match self.build(model, bytes, want, fp32) {
                Ok(s) => return Ok((s, want)),
                Err(e) => {
                    tracing::warn!(model = %name(model), device = ?want, error = %e.detail(), "GPU session failed; using the CPU")
                }
            }
        }
        Ok((self.build(model, bytes, Device::Cpu, fp32)?, Device::Cpu))
    }

    fn build(&self, model: &Path, bytes: Option<&[u8]>, device: Device, fp32: bool) -> InferResult<Session> {
        #[cfg(not(target_os = "macos"))]
        let _ = fp32;
        let err = |e: String| InferError::Model(format!("{}: {e}", model.display()));
        let mut b = Session::builder(&self.env).map_err(|e| err(e.to_string()))?;
        match device {
            #[cfg(windows)]
            Device::DirectMl => {
                // DirectML needs sequential execution without memory patterns.
                // The fastest GPU, not adapter 0: on a laptop that is often
                // the integrated one, with the dedicated GPU left idle.
                let dml = ort::ep::DirectML::default()
                    .with_performance_preference(ort::ep::directml::PerformancePreference::HighPerformance);
                b = b
                    .with_execution_providers([dml.build().error_on_failure()])
                    .map_err(|e| err(e.to_string()))?
                    .with_memory_pattern(false)
                    .map_err(|e| err(e.to_string()))?
                    .with_parallel_execution(false)
                    .map_err(|e| err(e.to_string()))?;
            }
            #[cfg(target_os = "macos")]
            Device::CoreMl => {
                // fp32 sessions run as an MLProgram without fp16
                // accumulation on the GPU (the anime models': their
                // embeddings bend under low-precision maths).
                let ep = match fp32 {
                    true => ort::ep::CoreML::default()
                        .with_model_format(ort::ep::coreml::ModelFormat::MLProgram)
                        .with_low_precision_accumulation_on_gpu(false),
                    false => ort::ep::CoreML::default(),
                };
                b = b.with_execution_providers([ep.build().error_on_failure()]).map_err(|e| err(e.to_string()))?;
            }
            _ => {}
        }
        match bytes {
            Some(bytes) => b.commit_from_memory(bytes).map_err(|e| err(e.to_string())),
            None => b.commit_from_file(model).map_err(|e| err(e.to_string())),
        }
    }
}

/// What [`Runtime::fp32_sessions`] returns: the same sessions — same
/// signatures — but built so CoreML keeps fp32 maths. Off CoreML there is
/// nothing to build differently.
pub struct Fp32<'a> {
    rt: &'a Runtime,
}

impl Fp32<'_> {
    /// Like [`Runtime::session`].
    pub fn session(&self, model: &Path) -> InferResult<(Session, Device)> {
        self.rt.session_from(model, None, true)
    }

    /// Like [`Runtime::session_shaped`].
    pub fn session_shaped(&self, model: &Path, input: onnx::InputShape) -> InferResult<(Session, Device)> {
        let bytes = std::fs::read(model).map_err(|e| InferError::Model(format!("{}: {e}", model.display())))?;
        let patched = onnx::reshaped(&bytes, input).ok_or_else(|| {
            InferError::Model(format!("{}: not a readable ONNX model, or not shaped like {input:?}", model.display()))
        })?;
        self.rt.session_from(model, Some(&patched), true)
    }

    /// Like [`Runtime::session_batched`].
    pub fn session_batched(&self, model: &Path, input: onnx::InputShape) -> InferResult<(Session, Device)> {
        let bytes = std::fs::read(model).map_err(|e| InferError::Model(format!("{}: {e}", model.display())))?;
        let unreadable = |what: &str| InferError::Model(format!("{}: {what}", model.display()));
        let (free, targets) = onnx::free_reshape_batch(&bytes)
            .ok_or_else(|| unreadable("not a readable ONNX model, or a Reshape that takes 0 literally"))?;
        let patched = onnx::reshaped(&free, input).ok_or_else(|| unreadable(&format!("not shaped like {input:?}")))?;
        tracing::debug!(model = %model.display(), targets, "Reshapes copy the batch");
        self.rt.session_from(model, Some(&patched), true)
    }
}

/// Just the file name, for the log: full paths name the user (their home
/// folder), and logs get shared.
fn name(path: &Path) -> std::borrow::Cow<'_, str> {
    path.file_name().map_or("?".into(), |n| n.to_string_lossy())
}

/// Make DirectML.dll resolve to the bundled copy: load it by full path (the
/// runtime's later `LoadLibrary("DirectML.dll")` then finds it already loaded)
/// and put the folder on the DLL search path for anything else it needs.
#[cfg(windows)]
fn preload_windows(dir: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{LoadLibraryW, SetDllDirectoryW};
    let wide = |p: &Path| p.as_os_str().encode_wide().chain([0]).collect::<Vec<u16>>();
    // SAFETY: valid NUL-terminated wide strings; the module stays loaded for
    // the life of the process, which is what the runtime expects.
    unsafe {
        SetDllDirectoryW(wide(dir).as_ptr());
        let dml = dir.join("DirectML.dll");
        if dml.is_file() && LoadLibraryW(wide(&dml).as_ptr()).is_null() {
            tracing::warn!(path = %name(&dml), "couldn't preload DirectML.dll");
        }
    }
}

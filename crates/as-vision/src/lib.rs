//! Vision models on top of [`as_infer`]: TransNetV2 shot boundaries, SCRFD
//! face detection and ArcFace identity embeddings.

pub mod arcface;
pub mod image;
pub mod scrfd;
pub mod transnet;

pub use arcface::{ArcFace, Embedding};
pub use scrfd::{FaceBox, Scrfd};
pub use transnet::{TransNet, TransNetStream};

//! Reusable ffmpeg filter snippets.

use crate::{HdrKind, MediaInfo};

/// HDR (PQ/HLG) → SDR BT.709 with Hable tone mapping. Identical on every
/// platform (zimg), so posters, proxies and exports look the same everywhere.
pub const TONEMAP_SDR: &str =
    "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv";

/// Filters that turn decoded frames into what a viewer should see, without
/// changing the number of frames:
/// - deinterlace one-frame-per-frame (`bwdif send_frame`) for interlaced sources,
/// - tone map HDR to SDR when requested,
/// - convert anamorphic pixels to square,
/// - scale to `height` (never upscale) with even dimensions.
pub fn display_chain(info: &MediaInfo, height: Option<u32>, tonemap: bool) -> Vec<String> {
    let mut f = Vec::new();
    if info.is_interlaced() {
        f.push("bwdif=mode=send_frame".to_string());
    }
    if tonemap && info.video.hdr != HdrKind::None {
        f.push(TONEMAP_SDR.to_string());
    }
    let sar = info.video.sar;
    if sar.num != sar.den {
        f.push("scale=trunc(iw*sar/2)*2:ih,setsar=1".to_string());
    }
    if let Some(h) = height {
        f.push(format!("scale=-2:'min({h},ih)':flags=bicubic"));
    }
    f
}

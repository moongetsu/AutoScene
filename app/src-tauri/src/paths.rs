//! Paths that come in from the webview (dropped and picked files, export and
//! library folders, a file to show in Explorer) and the ones the asset
//! protocol is opened to.
//!
//! Only plain paths are taken: absolute, on a drive of this computer or a
//! network share (`\\nas\share`, where libraries legitimately live), and
//! never climbing out with `..`. Device and verbatim paths (`\\.\…`,
//! `\\?\…`) get past the file system's usual rules, `\\?\GLOBALROOT\…` to
//! any device at all; neither the system's dialogs nor AutoScene spell a
//! path that way. A relative path would depend on the working directory.

use std::borrow::Cow;
use std::path::{Component, Path};

use crate::error::{AppError, AppResult, ErrorCode};

/// Refuse `raw` unless it's a plain path (see the module docs).
pub fn check(raw: impl AsRef<Path>) -> AppResult<()> {
    if is_plain(raw.as_ref()) {
        return Ok(());
    }
    log::warn!("refused a path from the page: relative, a device's, verbatim or climbing out with `..`");
    Err(AppError::new(
        ErrorCode::Invalid,
        "AutoScene can't use that location. Choose a file or folder on this computer or on a network share.",
    ))
}

/// [`check`] every path of a list (a drop, a pick of several files).
pub fn check_all(raw: &[String]) -> AppResult<()> {
    raw.iter().try_for_each(check)
}

/// Absolute, on a drive or a network share, and without `..`.
pub fn is_plain(path: &Path) -> bool {
    path.is_absolute()
        && plain_prefix(path)
        && !device_spelling(path)
        && !path.components().any(|c| c == Component::ParentDir)
}

/// A device or verbatim prefix spelt with a slash somewhere (`//?/C:/…`,
/// `\\?/…`, `//./…`): Rust reads those as a share named `?` or `.`, Windows
/// as the device paths they are.
fn device_spelling(path: &Path) -> bool {
    let s = path.as_os_str().to_string_lossy();
    let mut chars = s.chars();
    let (a, b, c, d) = (chars.next(), chars.next(), chars.next(), chars.next());
    let slash = |c: Option<char>| matches!(c, Some('/' | '\\'));
    slash(a) && slash(b) && matches!(c, Some('?' | '.')) && (slash(d) || d.is_none())
}

/// A drive (`C:\`) or a share (`\\nas\share`): on Windows every absolute
/// path starts with one of these or a device/verbatim prefix.
#[cfg(windows)]
fn plain_prefix(path: &Path) -> bool {
    use std::path::Prefix;
    match path.components().next() {
        Some(Component::Prefix(p)) => matches!(p.kind(), Prefix::Disk(_) | Prefix::UNC(..)),
        _ => false,
    }
}

#[cfg(not(windows))]
fn plain_prefix(_: &Path) -> bool {
    true
}

/// Whether `path` is `root` or lies inside it, compared part by part (so
/// `C:\Lib2` isn't inside `C:\Lib`). Both must be plain: a `..` could climb
/// back out, and an empty root would hold everything.
pub fn is_within(path: &Path, root: &Path) -> bool {
    is_plain(path) && is_plain(root) && path.starts_with(root)
}

/// Just the file (or folder) name, for the log: full paths name the user
/// (their home folder) and their folders, and logs get shared.
pub fn name(path: &Path) -> Cow<'_, str> {
    path.file_name().map_or(Cow::Borrowed("?"), |n| n.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn drives_and_network_shares_are_plain_device_and_verbatim_paths_are_not() {
        for bad in
            [r"//?/C:/Videos/x.mkv", r"\\?/C:/x.mkv", "//./PhysicalDrive0", r"//?\GLOBALROOT/Device", r"\\./COM1"]
        {
            assert!(!is_plain(Path::new(bad)), "{bad}");
        }
        for ok in
            [r"C:\Videos\S01E01.mkv", r"d:\Exports", "C:/Videos/S01E01.mkv", r"\\nas\share\x.mkv", "//nas/share/x.mkv"]
        {
            assert!(is_plain(Path::new(ok)), "{ok}");
            assert!(check(ok).is_ok(), "{ok}");
        }
        for bad in [
            r"\\?\C:\Videos\S01E01.mkv",
            r"\\?\UNC\nas\share\x.mkv",
            r"\\.\UNC\nas\share\x.mkv",
            r"\\.\PhysicalDrive0",
            r"\\?\GLOBALROOT\Device\Mup\nas\share",
            r"\??\UNC\nas\share",
            r"\\server",
            r"C:Videos\x.mkv",
            r"\Videos\x.mkv",
            r"Videos\x.mkv",
            r"C:\Library\..\Windows",
            r"\\nas\share\..\x",
            "",
        ] {
            assert!(!is_plain(Path::new(bad)), "{bad}");
            assert!(matches!(check(bad), Err(AppError { code: ErrorCode::Invalid, .. })), "{bad}");
        }
        assert!(check_all(&[r"C:\a.mkv".into(), r"\\nas\s\b.mkv".into()]).is_ok());
        assert!(check_all(&[r"C:\a.mkv".into(), r"\\.\UNC\nas\s\b.mkv".into()]).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn absolute_paths_are_plain_relative_and_climbing_ones_are_not() {
        assert!(is_plain(Path::new("/Users/me/Movies/S01E01.mkv")));
        assert!(check_all(&["/Volumes/NAS/Show/a.mkv".into(), "/home/me/b.mkv".into()]).is_ok());
        for bad in ["Movies/x.mkv", "./x.mkv", "/Users/me/../other/x.mkv", ""] {
            assert!(!is_plain(Path::new(bad)), "{bad}");
            assert!(check(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn within_compares_whole_parts_and_never_climbs_out() {
        let root = std::env::temp_dir().join("Library");
        assert!(is_within(&root, &root));
        assert!(is_within(&root.join("Show.autoscene").join("thumbs"), &root));
        assert!(!is_within(&std::env::temp_dir().join("Library2"), &root));
        assert!(!is_within(&root.join("..").join("elsewhere"), &root));
        assert!(!is_within(&root.join("x"), Path::new("")));
        assert!(!is_within(Path::new("x"), Path::new("x")));
    }

    #[test]
    fn the_log_gets_the_name_only() {
        assert_eq!(name(&std::env::temp_dir().join("Show").join("S01E01.mkv")), "S01E01.mkv");
        assert_eq!(name(Path::new("")), "?");
    }
}

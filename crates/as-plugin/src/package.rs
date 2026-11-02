//! Plugin packages. So far only the rule every path in a package meets; the
//! manifest uses it for the programs a command starts.

use crate::error::{PluginError, PluginResult};
use crate::manifest::shown;

/// The longest path a package may hold, in bytes.
pub const MAX_PATH_BYTES: usize = 200;
/// The most folders deep (counting the file) a package path may go.
pub const MAX_DEPTH: usize = 12;

/// Whether `p` may name a file in a package: relative, split on `/` only,
/// and spelled one way on every system. Each part keeps to ASCII letters,
/// digits and `._@+-`, isn't `.` or `..`, doesn't end in a dot (Windows
/// drops it), and isn't a device name Windows reserves (`NUL.txt` is the
/// console's null device there). That leaves out `\`, drive letters,
/// `~` short names, `:` streams and spaces, so no two such paths can name
/// the same file, nor one outside its folder.
pub fn check_path(p: &str) -> PluginResult<()> {
    let refuse = |why: &str| {
        Err(PluginError::Invalid(format!("The plugin names a file that can't be used: {} ({why}).", shown(p))))
    };
    if p.is_empty() {
        return refuse("it's empty");
    }
    if p.len() > MAX_PATH_BYTES {
        return refuse("the path is too long");
    }
    let parts: Vec<&str> = p.split('/').collect();
    if parts.len() > MAX_DEPTH {
        return refuse("too many folders deep");
    }
    for part in parts {
        if part.is_empty() {
            return refuse("it must be a relative path without empty parts");
        }
        if part == "." || part == ".." {
            return refuse("it can't step into another folder");
        }
        if !part.bytes().all(|b| b.is_ascii_alphanumeric() || b"._@+-".contains(&b)) {
            return refuse("use only letters, digits and . _ @ + -");
        }
        if part.ends_with('.') {
            return refuse("a name can't end with a dot");
        }
        if reserved(part) {
            return refuse("Windows reserves that name");
        }
    }
    Ok(())
}

/// Whether Windows takes a file of this name for a device (`CON`, `NUL`,
/// `COM1`, also with an extension).
pub(crate) fn reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or_default().to_ascii_uppercase();
    match stem.as_bytes() {
        b"CON" | b"PRN" | b"AUX" | b"NUL" => true,
        [b'C', b'O', b'M', d] | [b'L', b'P', b'T', d] => d.is_ascii_digit(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_paths_are_relative_and_spelled_one_way() {
        for ok in ["plugin.json", "bin/tool.exe", "lib/a-b_c@2+x/main.py", ".hidden", "CONFIG.txt", "com10"] {
            assert!(check_path(ok).is_ok(), "{ok}");
        }
        let long = "a/".repeat(MAX_DEPTH) + "b";
        let wide = "a".repeat(MAX_PATH_BYTES + 1);
        for bad in [
            "",
            "/abs",
            "C:x",
            "a/../b",
            "../evil",
            "./a",
            "a//b",
            "a/",
            r"bin\x.exe",
            "a b",
            "x:ads",
            "PLUGIN~1.JSO",
            "plugin.json.",
            "NUL.txt",
            "con",
            "lpt1.log",
            "nämlich.txt",
            &long,
            &wide,
        ] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
    }
}

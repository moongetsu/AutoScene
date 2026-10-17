//! Just enough of the zip format to fetch one file of an archive by its byte
//! ranges: the end-of-central-directory record at the archive's end, the
//! central directory it points at, and the member's local header, after
//! which its data follows. ZIP64, split and encrypted archives are refused.

use std::io::{Read, Write};

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;
/// The end record without its comment.
const EOCD_LEN: usize = 22;
const CENTRAL_LEN: usize = 46;
/// A local header without its name and extra field.
pub const LOCAL_LEN: u64 = 30;
/// The archive's last bytes that surely hold its end record (whose comment
/// is at most 64 KiB).
pub const TAIL: u64 = EOCD_LEN as u64 + 0xFFFF;
/// The biggest central directory read (thousands of members): an archive
/// claiming more isn't one to download from.
pub const MAX_DIRECTORY: u64 = 4 << 20;

/// The most bytes a member of `size` bytes takes in an archive: deflate adds
/// a few bytes per 16 KiB block to what it can't shrink.
pub fn packed_at_most(size: u64) -> u64 {
    size + size / 64 + 1024
}

/// How a member is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Stored,
    Deflated,
}

/// A member, as the central directory describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub method: Method,
    /// Bytes in the archive, and once unpacked.
    pub compressed: u64,
    pub size: u64,
    /// Where its local header starts.
    pub header: u64,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Where the central directory is — `(offset, size)` in the archive — from
/// its last bytes (`tail`, the archive's end).
pub fn directory(tail: &[u8]) -> Result<(u64, u64), String> {
    // The end record whose comment reaches exactly to the end: the comment
    // itself may hold anything, its signature too.
    let at = (0..=tail.len().saturating_sub(EOCD_LEN))
        .rev()
        .find(|&i| {
            u32_at(tail, i) == Some(EOCD_SIG)
                && u16_at(tail, i + 20).is_some_and(|c| i + EOCD_LEN + usize::from(c) == tail.len())
        })
        .ok_or("no end of the central directory: not a zip archive")?;
    let r = &tail[at..];
    let (disk, cd_disk) = (u16_at(r, 4).unwrap_or(1), u16_at(r, 6).unwrap_or(1));
    let size = u32_at(r, 12).ok_or("truncated end record")?;
    let offset = u32_at(r, 16).ok_or("truncated end record")?;
    if disk != 0 || cd_disk != 0 {
        return Err("a split archive".into());
    }
    if size == u32::MAX || offset == u32::MAX {
        return Err("a ZIP64 archive".into());
    }
    if u64::from(size) > MAX_DIRECTORY {
        return Err(format!("a central directory of {size} bytes"));
    }
    Ok((u64::from(offset), u64::from(size)))
}

/// The member `name` of a central directory.
pub fn find(dir: &[u8], name: &str) -> Result<Entry, String> {
    let mut at = 0;
    while at + CENTRAL_LEN <= dir.len() {
        if u32_at(dir, at) != Some(CENTRAL_SIG) {
            return Err(format!("damaged central directory at {at}"));
        }
        let field16 = |off: usize| u16_at(dir, at + off).map(usize::from).unwrap_or(0);
        let field32 = |off: usize| u32_at(dir, at + off).unwrap_or(0);
        let (name_len, extra_len, comment_len) = (field16(28), field16(30), field16(32));
        let this = dir.get(at + CENTRAL_LEN..at + CENTRAL_LEN + name_len).ok_or("truncated central directory")?;
        if this == name.as_bytes() {
            if field16(8) & 1 != 0 {
                return Err(format!("{name} is encrypted"));
            }
            let method = match field16(10) {
                0 => Method::Stored,
                8 => Method::Deflated,
                m => return Err(format!("{name} is compressed with method {m}")),
            };
            let (compressed, size, header) = (field32(20), field32(24), field32(42));
            if [compressed, size, header].contains(&u32::MAX) {
                return Err(format!("{name} needs ZIP64"));
            }
            return Ok(Entry {
                method,
                compressed: u64::from(compressed),
                size: u64::from(size),
                header: u64::from(header),
            });
        }
        at += CENTRAL_LEN + name_len + extra_len + comment_len;
    }
    Err(format!("{name} isn't in the archive"))
}

/// Bytes from a member's local header (its first [`LOCAL_LEN`] bytes) to
/// its data.
pub fn data_offset(local: &[u8]) -> Result<u64, String> {
    if u32_at(local, 0) != Some(LOCAL_SIG) {
        return Err("no local header where the directory says".into());
    }
    let (name, extra) = (u16_at(local, 26).ok_or("truncated local header")?, u16_at(local, 28).unwrap_or(0));
    Ok(LOCAL_LEN + u64::from(name) + u64::from(extra))
}

/// Unpack a member's data (`packed`) into `out`, at most `limit` bytes,
/// returning how many it gave (data that unpacks to more stops there).
pub fn unpack(method: Method, packed: impl Read, out: &mut impl Write, limit: u64) -> std::io::Result<u64> {
    match method {
        Method::Stored => std::io::copy(&mut packed.take(limit), out),
        Method::Deflated => std::io::copy(&mut flate2::read::DeflateDecoder::new(packed).take(limit), out),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A zip archive of `files` (name, content, deflated?), with a comment.
    pub(crate) fn archive(files: &[(&str, &[u8], bool)], comment: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for &(name, data, deflate) in files {
            let packed = if deflate {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let header = out.len() as u32;
            let method: u16 = if deflate { 8 } else { 0 };
            let extra = b"\x99\x99\x02\x00ab"; // an unknown extra field the reader must skip
            out.extend_from_slice(&LOCAL_SIG.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 8]); // time, date, crc (unchecked: the pin is a SHA256)
            out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(extra);
            out.extend_from_slice(&packed);
            central.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            central.extend_from_slice(&header.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let (cd_offset, cd_size) = (out.len() as u32, central.len() as u32);
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        out.extend_from_slice(comment);
        out
    }

    #[test]
    fn a_member_is_found_by_its_ranges_and_unpacked() {
        let model: Vec<u8> = (0..50_000u32).map(|i| (i * 7 % 251) as u8).collect();
        // A comment holding the end record's signature must not fool the search.
        let zip = archive(
            &[("manifest.json", b"{}", false), ("model.onnx", &model, true), ("raw.bin", b"stored", false)],
            b"PK\x05\x06 is not where the archive's end record is, it's only a comment",
        );
        let tail = &zip[zip.len().saturating_sub(TAIL as usize)..];
        let (offset, size) = directory(tail).unwrap();
        let dir = &zip[offset as usize..(offset + size) as usize];
        for (name, content, method) in
            [("model.onnx", &model[..], Method::Deflated), ("raw.bin", b"stored", Method::Stored)]
        {
            let e = find(dir, name).unwrap();
            assert_eq!((e.method, e.size), (method, content.len() as u64));
            let start = e.header + data_offset(&zip[e.header as usize..(e.header + LOCAL_LEN) as usize]).unwrap();
            assert!(e.compressed <= packed_at_most(e.size));
            let packed = &zip[start as usize..(start + e.compressed) as usize];
            let mut out = Vec::new();
            let n = unpack(e.method, packed, &mut out, e.size + 1).unwrap();
            assert_eq!((n, &out[..]), (content.len() as u64, content));
            // Data that unpacks to more than it should stops at the limit.
            let mut short = Vec::new();
            assert_eq!(unpack(e.method, packed, &mut short, 4).unwrap(), 4);
            assert_eq!(short, content[..4]);
        }
        assert!(find(dir, "missing.onnx").unwrap_err().contains("isn't in the archive"));
        assert!(directory(b"not a zip at all, just bytes").is_err());
        // An end record claiming a huge directory is refused before it is read.
        let mut huge = archive(&[("a.bin", b"x", false)], b"");
        let end = huge.len() - EOCD_LEN;
        huge[end + 12..end + 16].copy_from_slice(&(MAX_DIRECTORY as u32 + 1).to_le_bytes());
        assert!(directory(&huge).unwrap_err().contains("central directory of"));
        assert!(data_offset(&[0; 30]).is_err());
    }
}

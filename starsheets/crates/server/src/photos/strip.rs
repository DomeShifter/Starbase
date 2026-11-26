//! A copy of a picture without where it was taken, for people who aren't its owner (a share link,
//! a download by someone it's shared with). Nothing is re-encoded: the pixels are the original's
//! bytes. A JPEG keeps its other segments; its EXIF is rebuilt from the same allow-list the export
//! uses (camera, date, exposure; orientation as stored), and XMP and Photoshop blocks are left out.
//! A PNG loses its EXIF and text chunks (where XMP lives).

use super::export;

/// Most bytes looked at; a bigger file is not copied here (the caller sends nothing).
pub const MAX: u64 = super::MAX_FILE;

/// `bytes` without location data, for a JPEG or PNG; None for anything else or a broken file.
pub fn without_location(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.starts_with(&[0xFF, 0xD8]) {
        jpeg(bytes)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(bytes)
    } else {
        None
    }
}

fn jpeg(b: &[u8]) -> Option<Vec<u8>> {
    let mut out = vec![0xFF, 0xD8];
    let mut at = 2;
    loop {
        if *b.get(at)? != 0xFF {
            return None;
        }
        let m = *b.get(at + 1)?;
        if m == 0xFF {
            at += 1; // fill byte
            continue;
        }
        // Standalone markers, then the scan: the rest is the picture, copied as it is.
        if m == 0xDA || m == 0xD9 {
            out.extend_from_slice(&b[at..]);
            break;
        }
        if m == 0x01 || (0xD0..=0xD7).contains(&m) {
            out.extend_from_slice(&b[at..at + 2]);
            at += 2;
            continue;
        }
        let len = usize::from(u16::from_be_bytes([*b.get(at + 2)?, *b.get(at + 3)?]));
        let end = at + 2 + len;
        if len < 2 || end > b.len() {
            return None;
        }
        // APP1 (EXIF, XMP) and APP13 (Photoshop/IPTC) are dropped; the rest stays.
        if m != 0xE1 && m != 0xED {
            out.extend_from_slice(&b[at..end]);
        }
        at = end;
    }
    let o = crate::photos::decode::exif_meta(b).orientation;
    Some(match export::exif_block_turned(b, false, o) {
        Some(tiff) => export::with_exif(out, &tiff),
        None => out,
    })
}

fn png(b: &[u8]) -> Option<Vec<u8>> {
    let mut out = b[..8].to_vec();
    let mut at = 8;
    while at < b.len() {
        let len = u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize;
        let end = at.checked_add(12)?.checked_add(len)?;
        if end > b.len() {
            return None;
        }
        if !matches!(&b[at + 4..at + 8], b"eXIf" | b"iTXt" | b"tEXt" | b"zTXt") {
            out.extend_from_slice(&b[at..end]);
        }
        at = end;
    }
    Some(out)
}

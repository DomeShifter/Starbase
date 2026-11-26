//! The JPEG preview inside a camera RAW file (DNG, CR2, NEF, ARW, ORF, PEF, SRW, RW2, RAF).
//!
//! Most RAW files are TIFF containers, and nearly all carry a full-size or large JPEG next to the
//! sensor data. Thumbnails are made from that, so no RAW decoder is needed. Every offset is
//! checked against the file's length, only a few IFDs are followed (bounded work), and the JPEG
//! found is just a slice of the input.

/// The most IFDs looked at in one file.
const MAX_IFDS: usize = 32;

struct Tiff<'a> {
    b: &'a [u8],
    le: bool,
}

impl Tiff<'_> {
    fn u16(&self, at: usize) -> Option<u32> {
        let s: [u8; 2] = self.b.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(u32::from(if self.le { u16::from_le_bytes(s) } else { u16::from_be_bytes(s) }))
    }
    fn u32(&self, at: usize) -> Option<u32> {
        let s: [u8; 4] = self.b.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if self.le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    }
}

/// The largest embedded JPEG of a RAW file, if it has one.
pub fn preview(b: &[u8]) -> Option<&[u8]> {
    // Fujifilm RAF: own header; the JPEG's offset and length (big-endian) are at 84 and 88.
    if b.starts_with(b"FUJIFILMCCD-RAW") {
        let off = u32::from_be_bytes(b.get(84..88)?.try_into().ok()?) as usize;
        let len = u32::from_be_bytes(b.get(88..92)?.try_into().ok()?) as usize;
        return jpeg_at(b, off, len);
    }
    let le = match b.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let t = Tiff { b, le };
    // 42 = TIFF; 0x4F52 / 0x5352 / 0x55 = Olympus ORF / RW2, the same layout.
    if !matches!(t.u16(2)?, 42 | 0x4F52 | 0x5352 | 0x55) {
        return None;
    }
    let mut todo = vec![t.u32(4)? as usize];
    let mut seen = 0;
    let mut best: Option<&[u8]> = None;
    while let Some(ifd) = todo.pop() {
        seen += 1;
        if seen > MAX_IFDS {
            break;
        }
        if ifd == 0 {
            continue;
        }
        let Some(n) = t.u16(ifd) else { continue };
        let (mut jpeg_off, mut jpeg_len, mut strip_off, mut strip_len, mut compression) = (None, None, None, None, 0);
        for i in 0..n as usize {
            let e = ifd + 2 + i * 12;
            let (Some(tag), Some(kind)) = (t.u16(e), t.u16(e + 2)) else { break };
            // A single SHORT or LONG value is stored in the entry itself.
            let value = if kind == 3 { t.u16(e + 8) } else { t.u32(e + 8) };
            match tag {
                0x103 => compression = value.unwrap_or(0),
                0x111 => strip_off = value, // first strip only: previews are one strip
                0x117 => strip_len = value,
                0x201 => jpeg_off = value,
                0x202 => jpeg_len = value,
                // SubIFDs: one offset inline, several in an array.
                0x14A => {
                    let count = t.u32(e + 4).unwrap_or(0).min(8) as usize;
                    if count == 1 {
                        todo.extend(t.u32(e + 8).map(|v| v as usize));
                    } else if let Some(arr) = t.u32(e + 8) {
                        todo.extend((0..count).filter_map(|k| t.u32(arr as usize + k * 4).map(|v| v as usize)));
                    }
                }
                _ => {}
            }
        }
        let mut found = None;
        if let (Some(o), Some(l)) = (jpeg_off, jpeg_len) {
            found = jpeg_at(b, o as usize, l as usize);
        }
        if found.is_none()
            && matches!(compression, 6 | 7)
            && let (Some(o), Some(l)) = (strip_off, strip_len)
        {
            found = jpeg_at(b, o as usize, l as usize);
        }
        if let Some(j) = found
            && best.is_none_or(|x| j.len() > x.len())
        {
            best = Some(j);
        }
        // The next IFD in the chain (IFD1 holds a small preview in CR2/NEF).
        if let Some(next) = t.u32(ifd + 2 + n as usize * 12) {
            todo.push(next as usize);
        }
    }
    best
}

fn jpeg_at(b: &[u8], off: usize, len: usize) -> Option<&[u8]> {
    let s = b.get(off..off.checked_add(len)?)?;
    s.starts_with(&[0xFF, 0xD8, 0xFF]).then_some(s)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A tiny valid JPEG, made with the encoder.
    pub fn small_jpeg() -> Vec<u8> {
        let mut out = Vec::new();
        let px: Vec<u8> = (0..16 * 8 * 3).map(|i| (i % 251) as u8).collect();
        image::codecs::jpeg::JpegEncoder::new(&mut out).encode(&px, 16, 8, image::ExtendedColorType::Rgb8).unwrap();
        out
    }

    /// A little-endian TIFF (DNG-like): IFD0 holds a SubIFD with the JPEG as one strip.
    pub fn fake_dng(jpeg: &[u8]) -> Vec<u8> {
        let mut f = b"II*\0".to_vec();
        f.extend(8u32.to_le_bytes()); // IFD0 at 8
        // IFD0: one entry (SubIFDs, LONG, 1, offset 26), next = 0. 2 + 12 + 4 = 18 → ends at 26.
        f.extend(1u16.to_le_bytes());
        for v in [0x14Au16, 4] {
            f.extend(v.to_le_bytes());
        }
        f.extend(1u32.to_le_bytes());
        f.extend(26u32.to_le_bytes());
        f.extend(0u32.to_le_bytes());
        // SubIFD at 26: Compression 7, StripOffsets, StripByteCounts. 2 + 36 + 4 = 42 → JPEG at 26 + 42 = 68.
        f.extend(3u16.to_le_bytes());
        for (tag, kind, v) in [(0x103u16, 3u16, 7u32), (0x111, 4, 68), (0x117, 4, jpeg.len() as u32)] {
            f.extend(tag.to_le_bytes());
            f.extend(kind.to_le_bytes());
            f.extend(1u32.to_le_bytes());
            f.extend(v.to_le_bytes());
        }
        f.extend(0u32.to_le_bytes());
        assert_eq!(f.len(), 68);
        f.extend(jpeg);
        f
    }

    #[test]
    fn finds_the_preview_in_a_subifd() {
        let j = small_jpeg();
        assert_eq!(preview(&fake_dng(&j)), Some(&j[..]));
    }

    #[test]
    fn junk_and_truncated_files_give_nothing() {
        let j = small_jpeg();
        let f = fake_dng(&j);
        assert_eq!(preview(&f[..50]), None);
        assert_eq!(preview(b"II*\0\xff\xff\xff\xff"), None);
        assert_eq!(preview(b"not a raw file"), None);
        // An IFD whose next-IFD pointer is itself ends (bounded), it doesn't loop.
        let mut l = b"II*\0".to_vec();
        l.extend(8u32.to_le_bytes());
        l.extend(0u16.to_le_bytes());
        l.extend(8u32.to_le_bytes());
        assert_eq!(preview(&l), None);
    }

    #[test]
    fn thumbnails_come_from_the_preview() {
        let made = crate::photos::decode::make(&fake_dng(&small_jpeg()), &crate::photos::decode::Limits::for_memory(512 << 20)).unwrap();
        assert!(!made.thumbs.is_empty());
        assert_eq!((made.meta.width, made.meta.height), (16, 8));
    }
}

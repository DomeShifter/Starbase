//! Videos in Photos: what the timeline shows of one (its length and when it was shot), read from
//! the file's own `mvhd` box (MP4, M4V, MOV). This small read is cached in memory by (path, size,
//! mtime). A separate FFmpeg process can extract a poster when one is requested.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

/// Seconds between 1904-01-01 (QuickTime's zero) and 1970-01-01.
const EPOCH_1904: u64 = 2_082_844_800;

const POSTER_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_POSTER: u64 = 512 * 1024;
const POSTER_MEMORY: libc::rlim_t = 768 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    pub duration: Option<f64>,
    /// Creation time, seconds since 1970 (None when the file says 0).
    pub created: Option<i64>,
}

/// Whether a media type from [`super::media_of`] is a video.
pub fn is_video(media: &str) -> bool {
    media.starts_with("video/")
}

/// Extracts a single, aspect-preserving PNG from an already checked, open file.
pub fn extract_poster(mut file: File) -> Result<Vec<u8>, String> {
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Err("The video must be a regular file.".into());
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    // fd: can seek in a regular file, unlike pipe:0. No path or network protocols are allowed.
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-nostdin",
            "-v",
            "error",
            "-max_alloc",
            "134217728",
            "-protocol_whitelist",
            "fd,pipe",
            "-threads",
            "1",
            "-max_pixels",
            "33554432",
            "-fd",
            "0",
            "-i",
            "fd:",
            "-map",
            "0:V:0",
            "-an",
            "-sn",
            "-dn",
            "-filter_threads",
            "1",
            "-vf",
            "scale=256:256:force_original_aspect_ratio=decrease",
            "-frames:v",
            "1",
            "-c:v",
            "png",
            "-threads",
            "1",
            "-f",
            "image2pipe",
            "pipe:1",
        ])
        .stdin(Stdio::from(file))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear();
    // SAFETY: the post-fork hook uses only system calls to cap the child's memory and priority.
    unsafe {
        command.pre_exec(|| {
            let limit = libc::rlimit { rlim_cur: POSTER_MEMORY, rlim_max: POSTER_MEMORY };
            if libc::setrlimit(libc::RLIMIT_AS, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::setpriority(libc::PRIO_PROCESS, 0, 15);
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|e| format!("Couldn't start FFmpeg: {e}"))?;

    let deadline = Instant::now() + POSTER_TIMEOUT;
    let result = (|| -> Result<Vec<u8>, String> {
        let stdout = child.stdout.take().ok_or("No video poster output.")?;
        let (send, receive) = mpsc::channel();
        // Drain the pipe while the child is running, including when a frame fills its buffer.
        let _reader = std::thread::Builder::new()
            .spawn(move || {
                let mut bytes = Vec::new();
                let got = stdout.take(MAX_POSTER + 1).read_to_end(&mut bytes);
                let _ = send.send(got.map(|_| bytes));
            })
            .map_err(|e| e.to_string())?;
        let png = receive
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| "FFmpeg took too long.")?
            .map_err(|_| "Couldn't read the video poster.")?;
        if png.len() as u64 > MAX_POSTER {
            return Err("The video poster is too large.".into());
        }
        loop {
            match child.try_wait().map_err(|e| e.to_string())? {
                Some(status) if status.success() => break,
                Some(_) => return Err("FFmpeg couldn't read a video frame.".into()),
                None if Instant::now() >= deadline => return Err("FFmpeg took too long.".into()),
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        let (w, h) = crate::thumbs::png_size(&png)?;
        if w == 0 || h == 0 || w.max(h) > 256 {
            return Err("The video poster has an invalid size.".into());
        }
        Ok(png)
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

/// Length and creation time of an MP4/MOV; None for anything else (WebM) or a broken file.
pub fn read_info(f: &mut (impl Read + Seek)) -> Option<Info> {
    let end = f.seek(SeekFrom::End(0)).ok()?;
    find_mvhd(f, 0, end, 0)
}

fn header(f: &mut (impl Read + Seek), at: u64, end: u64) -> Option<([u8; 4], u64, u64)> {
    f.seek(SeekFrom::Start(at)).ok()?;
    let mut h = [0u8; 8];
    f.read_exact(&mut h).ok()?;
    let (mut size, mut hl) = (u32::from_be_bytes(h[..4].try_into().ok()?) as u64, 8u64);
    if size == 1 {
        let mut b = [0u8; 8];
        f.read_exact(&mut b).ok()?;
        (size, hl) = (u64::from_be_bytes(b), 16);
    } else if size == 0 {
        size = end - at;
    }
    (size >= hl && at + size <= end).then_some((h[4..8].try_into().ok()?, size, hl))
}

fn find_mvhd(f: &mut (impl Read + Seek), mut at: u64, end: u64, depth: u8) -> Option<Info> {
    for _ in 0..64 {
        if at + 8 > end {
            return None;
        }
        let (kind, size, hl) = header(f, at, end)?;
        match &kind {
            b"moov" if depth == 0 => {
                if let Some(i) = find_mvhd(f, at + hl, at + size, 1) {
                    return Some(i);
                }
            }
            b"mvhd" if depth == 1 => {
                f.seek(SeekFrom::Start(at + hl)).ok()?;
                let mut b = [0u8; 32];
                let n = f.read(&mut b).ok()?;
                let b = &b[..n];
                let (created, scale, dur) = match b.first()? {
                    0 if n >= 20 => (
                        u32::from_be_bytes(b[4..8].try_into().ok()?) as u64,
                        u32::from_be_bytes(b[12..16].try_into().ok()?),
                        u32::from_be_bytes(b[16..20].try_into().ok()?) as u64,
                    ),
                    1 if n >= 32 => (
                        u64::from_be_bytes(b[4..12].try_into().ok()?),
                        u32::from_be_bytes(b[20..24].try_into().ok()?),
                        u64::from_be_bytes(b[24..32].try_into().ok()?),
                    ),
                    _ => return None,
                };
                return Some(Info {
                    duration: (scale > 0).then(|| dur as f64 / scale as f64),
                    created: created.checked_sub(EPOCH_1904).filter(|c| *c > 0).map(|c| c as i64),
                });
            }
            _ => {}
        }
        at += size;
    }
    None
}

type Cache = Mutex<HashMap<(String, u64, i64), Option<Info>>>;

/// [`read_info`] of the file at `path`, remembered while the file is unchanged.
pub fn info_of(store: &crate::store::Store, path: &str, size: u64, mtime_ns: i64) -> Option<Info> {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = (path.to_string(), size, mtime_ns);
    if let Some(v) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return *v;
    }
    let got = store.open_safe(path).and_then(|mut f| read_info(&mut f));
    let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
    if c.len() > 20_000 {
        c.clear();
    }
    c.insert(key, got);
    got
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal MP4: ftyp, a free box, moov > mvhd (version 0): 1000 ticks/s, 12.5 s.
    fn tiny(created_1904: u32) -> Vec<u8> {
        let mut m = vec![0u8; 100];
        m[4..8].copy_from_slice(&created_1904.to_be_bytes());
        m[12..16].copy_from_slice(&1000u32.to_be_bytes());
        m[16..20].copy_from_slice(&12_500u32.to_be_bytes());
        let mut mvhd = ((m.len() + 8) as u32).to_be_bytes().to_vec();
        mvhd.extend(b"mvhd");
        mvhd.extend(&m);
        let mut moov = ((mvhd.len() + 8) as u32).to_be_bytes().to_vec();
        moov.extend(b"moov");
        moov.extend(mvhd);
        let mut out = 16u32.to_be_bytes().to_vec();
        out.extend(b"ftypisom\0\0\0\0");
        out.extend(12u32.to_be_bytes());
        out.extend(b"free\0\0\0\0");
        out.extend(moov);
        out
    }

    #[test]
    fn reads_length_and_date() {
        let b = tiny((EPOCH_1904 + 1_700_000_000) as u32);
        let i = read_info(&mut std::io::Cursor::new(b)).unwrap();
        assert_eq!(i, Info { duration: Some(12.5), created: Some(1_700_000_000) });
        assert_eq!(read_info(&mut std::io::Cursor::new(tiny(0))).unwrap().created, None);
        assert!(read_info(&mut std::io::Cursor::new(b"RIFFxxxxWEBM".to_vec())).is_none());
        assert!(read_info(&mut std::io::Cursor::new(Vec::new())).is_none());
    }
}

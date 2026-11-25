//! The real picture reader: `starsheets-server photo-worker`, as the server runs it for every
//! picture (src/photos/decode.rs). One picture on stdin, one JSON line + thumbnails on stdout.

use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

fn run(input: &[u8]) -> (serde_json::Value, Vec<u8>, std::process::ExitStatus) {
    run_with(input, &[])
}

/// With more arguments: the memory, then the job (JSON).
fn run_with(input: &[u8], args: &[&str]) -> (serde_json::Value, Vec<u8>, std::process::ExitStatus) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_starsheets-server"))
        .arg("photo-worker")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().unwrap();
    let nl = out.stdout.iter().position(|&b| b == b'\n').unwrap_or(out.stdout.len());
    (serde_json::from_slice(&out.stdout[..nl]).unwrap_or(serde_json::Value::Null), out.stdout.get(nl + 1..).unwrap_or_default().to_vec(), out.status)
}

#[test]
fn reads_a_picture_in_a_child_process() {
    let img = image::RgbImage::from_fn(1500, 1000, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 90]));
    let mut jpg = Cursor::new(Vec::new());
    img.write_to(&mut jpg, image::ImageFormat::Jpeg).unwrap();
    let (head, rest, status) = run(&jpg.into_inner());
    assert!(status.success());
    assert_eq!(head["ok"], true, "{head}");
    assert_eq!((head["meta"]["width"].as_u64(), head["meta"]["height"].as_u64()), (Some(1500), Some(1000)));
    let lens: Vec<u64> = head["thumbs"].as_array().unwrap().iter().map(|t| t[2].as_u64().unwrap()).collect();
    assert_eq!(lens.iter().sum::<u64>(), rest.len() as u64);
    let first = image::load_from_memory(&rest[..lens[0] as usize]).unwrap();
    assert_eq!((first.width(), first.height()), (256, 171));
}

#[test]
fn refuses_bombs_and_nonsense() {
    // A PNG header claiming 50000 × 50000 RGBA pixels (10 GB unpacked): refused before decoding.
    fn crc(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    let mut ihdr = b"IHDR".to_vec();
    ihdr.extend(50_000u32.to_be_bytes());
    ihdr.extend(50_000u32.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(13u32.to_be_bytes());
    png.extend(&ihdr);
    png.extend(crc(&ihdr).to_be_bytes());
    // An empty data chunk: the header is all there is.
    png.extend(0u32.to_be_bytes());
    png.extend(b"IDAT");
    png.extend(crc(b"IDAT").to_be_bytes());
    let (head, _, status) = run(&png);
    assert!(status.success());
    assert_eq!(head["ok"], false);
    assert!(head["error"].as_str().unwrap().contains("too big"), "{head}");
    let (head, _, _) = run(b"GIF89a\x01\x00\x01\x00 not really");
    assert_eq!(head["ok"], false, "{head}");
    let (head, _, _) = run(b"");
    assert_eq!(head["ok"], false, "{head}");
}

#[test]
fn renders_edits_and_exports_in_a_child_process() {
    let img = image::RgbImage::from_fn(600, 400, |x, _| if x < 300 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) });
    let mut jpg = Cursor::new(Vec::new());
    img.write_to(&mut jpg, image::ImageFormat::Jpeg).unwrap();
    let jpg = jpg.into_inner();
    let memory = (1536u64 << 20).to_string();
    // Thumbnails with the edits: a quarter turn, then the left half of that.
    let job = r#"{"edits": {"rot90": 1, "crop": {"l": 0, "t": 0, "r": 0.5, "b": 1}}}"#;
    let (head, rest, status) = run_with(&jpg, &[&memory, job]);
    assert!(status.success() && head["ok"] == true, "{head}");
    let first = image::load_from_memory(&rest[..head["thumbs"][0][2].as_u64().unwrap() as usize]).unwrap();
    assert_eq!((first.width(), first.height()), (85, 256), "600 × 400 turned: 400 × 600; its left half: 200 × 600; 256 high");
    // An export: one file.
    let job = r#"{"edits": {"rot90": 1}, "export": {"format": "png", "max_side": 300}}"#;
    let (head, rest, status) = run_with(&jpg, &[&memory, job]);
    assert!(status.success() && head["ok"] == true, "{head}");
    assert_eq!(head["thumbs"][0][2].as_u64(), Some(rest.len() as u64));
    let out = image::load_from_memory(&rest).unwrap();
    assert_eq!((out.width(), out.height()), (200, 300));
    // A job that makes no sense: refused before anything is read.
    let (_, _, status) = run_with(&jpg, &[&memory, r#"{"edits": {"exposure": 99}}"#]);
    assert_eq!(status.code(), Some(2));
}

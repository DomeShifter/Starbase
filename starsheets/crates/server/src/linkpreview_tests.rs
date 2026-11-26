use std::io::{Read, Write};
use std::net::TcpListener;

use super::*;

/// A tiny one-shot-per-connection web server on 127.0.0.1; `answer(path)` gives the whole reply.
fn serve(answer: impl Fn(&str) -> Vec<u8> + Send + 'static) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut s = s;
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
            let _ = s.write_all(&answer(&path));
        }
    });
    format!("http://{addr}")
}

fn page(body: &str) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_bytes()
}

#[test]
fn private_and_odd_addresses_are_not_public() {
    for a in [
        "127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "224.0.0.1", "255.255.255.255",
        "::1", "::", "fc00::1", "fe80::1", "::ffff:127.0.0.1", "::ffff:192.168.0.1", "::127.0.0.1", "64:ff9b::a00:1", "2002:c0a8:0101::1",
    ] {
        assert!(!is_public(a.parse().unwrap()), "{a} must be refused");
    }
    for a in ["1.1.1.1", "93.184.216.34", "2606:4700::1111", "::ffff:8.8.8.8"] {
        assert!(is_public(a.parse().unwrap()), "{a} is public");
    }
}

#[test]
fn only_public_web_addresses_pass_the_check() {
    assert!(check_url("https://example.com/a", false).is_ok());
    for bad in ["file:///etc/passwd", "ftp://example.com", "http://127.0.0.1/", "http://[::1]/", "http://192.168.0.5:8080/x", "http://localhost/", "javascript:alert(1)"] {
        assert!(check_url(bad, false).is_err(), "{bad}");
    }
}

#[test]
fn a_name_that_looks_up_to_a_private_address_is_refused() {
    // localhost resolves to 127.0.0.1 / ::1: the resolver must refuse it, not connect.
    let f = Fetcher::new(false);
    let r = f.agent().unwrap().get("http://localhost.:9/").call();
    assert!(r.is_err());
    assert!(f.get("http://localhost.:9/", MAX_PAGE).is_err());
}

#[test]
fn reads_title_site_picture_and_icon() {
    let html = r#"<html><head><title>Plain &amp; old</title>
        <meta property="og:title" content="Caf&eacute; &quot;Nice&quot; &#38; &#x41;">
        <meta property="og:site_name" content='The Site'>
        <meta name="twitter:image" content="/img/p.png">
        <link rel="shortcut icon" href="fav.png"></head><body><title>no</title>"#;
    let m = read_meta(html, "https://ex.com/a/b.html?x=1");
    assert_eq!(m.title, "Caf&eacute; \"Nice\" & A");
    assert_eq!(m.site, "The Site");
    assert_eq!(m.image.as_deref(), Some("https://ex.com/img/p.png"));
    assert_eq!(m.icon.as_deref(), Some("https://ex.com/a/fav.png"));
    let m = read_meta("<title>  Only\n title  </title>", "http://ex.com");
    assert_eq!(m.title, "Only title");
    assert_eq!(m.icon.as_deref(), Some("http://ex.com/favicon.ico"));
    assert!(read_meta(&format!("<title>{}</title>", "x".repeat(500)), "http://ex.com").title.chars().count() <= MAX_TEXT);
}

#[test]
fn joins_relative_addresses() {
    assert_eq!(join("https://a.com/x/y", "//b.com/z").as_deref(), Some("https://b.com/z"));
    assert_eq!(join("https://a.com/x/y", "/z").as_deref(), Some("https://a.com/z"));
    assert_eq!(join("https://a.com", "z").as_deref(), Some("https://a.com/z"));
    assert_eq!(join("https://a.com/x/", "data:image/png;base64,AA"), None);
    assert_eq!(join("https://a.com/x/", "javascript:x"), None);
}

#[test]
fn fetches_a_page_cuts_a_huge_one_and_refuses_a_redirect_inward() {
    let base = serve(|path| match path {
        "/" => page("<title>Hello</title>"),
        "/big" => page(&format!("<title>Big</title>{}", "x".repeat(5 << 20))),
        "/to-local" => b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        "/slow" => {
            std::thread::sleep(Duration::from_secs(8));
            page("<title>late</title>")
        }
        _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
    });
    let (v, _, _) = fetch_preview(&format!("{base}/"), true).unwrap();
    assert_eq!(v["title"], "Hello");
    let (_, _, b) = Fetcher::new(true).get(&format!("{base}/big"), MAX_PAGE).unwrap();
    assert_eq!(b.len() as u64, MAX_PAGE);
    // Without the test allowance the guard refuses the test server itself and the redirect.
    assert!(fetch_preview(&format!("{base}/"), false).is_err());
    assert!(Fetcher::new(false).get(&format!("{base}/to-local"), MAX_PAGE).is_err());
    let t = Instant::now();
    assert!(Fetcher::new(true).get(&format!("{base}/slow"), MAX_PAGE).is_err());
    assert!(t.elapsed() < Duration::from_secs(7));
}

#[test]
fn pictures_become_small_pngs_and_svg_is_refused() {
    let mut big = Vec::new();
    image::RgbaImage::from_pixel(900, 600, image::Rgba([1, 2, 3, 255])).write_to(&mut std::io::Cursor::new(&mut big), image::ImageFormat::Png).unwrap();
    let small = small_png(&big, PICTURE_PX).unwrap();
    let img = image::load_from_memory(&small).unwrap();
    assert_eq!((img.width(), img.height()), (320, 213));
    assert!(small_png(b"<svg xmlns='http://www.w3.org/2000/svg'/>", ICON_PX).is_none());
}

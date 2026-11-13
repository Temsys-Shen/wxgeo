use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub struct MapPick {
    pub result: Arc<Mutex<Option<(f64, f64)>>>,
    pub stop: Arc<AtomicBool>,
    pub port: u16,
}

const PAGE: &str = include_str!("../assets/map.html");

pub fn start() -> std::io::Result<MapPick> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    listener.set_nonblocking(true)?;
    let result: Arc<Mutex<Option<(f64, f64)>>> = Arc::new(Mutex::new(None));
    let stop = Arc::new(AtomicBool::new(false));
    let r2 = result.clone();
    let s2 = stop.clone();
    std::thread::spawn(move || {
        while !s2.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut c, _)) => { let r = r2.clone(); std::thread::spawn(move || { let _ = handle(&mut c, &r); }); }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(std::time::Duration::from_millis(50)),
                Err(_) => break,
            }
        }
    });
    Ok(MapPick { result, stop, port })
}

fn handle(c: &mut std::net::TcpStream, result: &Arc<Mutex<Option<(f64, f64)>>>) -> std::io::Result<()> {
    c.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = c.read(&mut tmp)?;
        if n == 0 { break; }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") { break; }
        if buf.len() > 65536 { break; }
    }
    let head_end = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4).unwrap_or(buf.len());
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let first = head.lines().next().unwrap_or("");
    if first.starts_with("POST /pick") {
        let clen: usize = head.lines().find_map(|l| {
            if l.to_lowercase().starts_with("content-length:") { l.split(':').nth(1).and_then(|v| v.trim().parse().ok()) } else { None }
        }).unwrap_or(0);
        let mut body = buf[head_end..].to_vec();
        while body.len() < clen {
            match c.read(&mut tmp) { Ok(0) => break, Ok(n) => body.extend_from_slice(&tmp[..n]), Err(_) => break }
        }
        let text = String::from_utf8_lossy(&body).to_string();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let lat = v.get("lat").and_then(|x| x.as_f64());
            let lon = v.get("lon").and_then(|x| x.as_f64());
            if let (Some(lat), Some(lon)) = (lat, lon) {
                *result.lock().unwrap() = Some((lat, lon));
            }
        }
        let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}");
    } else {
        let body = PAGE.as_bytes();
        let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        c.write_all(resp.as_bytes())?;
        c.write_all(body)?;
    }
    Ok(())
}

pub fn open_browser(url: &str) {
    if std::env::var_os("WXGEO_NO_BROWSER").is_some() { return; }
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg(url).status(); }
    #[cfg(target_os = "windows")]
    { let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).status(); }
    #[cfg(target_os = "linux")]
    { let _ = std::process::Command::new("xdg-open").arg(url).status(); }
}

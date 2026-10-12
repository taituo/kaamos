//! Tiny std-only HTTP server for the phone control panel.
//!   GET /             the panel
//!   GET /api/state    JSON status
//!   GET /api/cmd?c=…  run a command (same syntax as render --events), returns state
//!   GET /api/stream   server-sent events: the state ~12 times a second
//!   GET /api/play?n=… live lead: MIDI note to play, -1 to release

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use crate::params::Shared;

const PANEL: &str = include_str!("panel.html");

pub fn spawn(sh: Arc<Shared>, bind: &str, port: u16) -> Result<String, String> {
    let listener = TcpListener::bind((bind, port)).map_err(|e| format!("cannot listen on {bind}:{port}: {e}"))?;
    let host = if bind == "0.0.0.0" {
        local_ip().unwrap_or_else(|| "localhost".into())
    } else {
        bind.to_string()
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let sh = sh.clone();
            std::thread::spawn(move || {
                let _ = handle(stream, &sh);
            });
        }
    });
    Ok(format!("http://{host}:{port}"))
}

fn local_ip() -> Option<String> {
    // No packets are sent: connecting a UDP socket only selects the outgoing interface.
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    if ip.is_unspecified() || ip.is_loopback() {
        None
    } else {
        Some(ip.to_string())
    }
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let h = std::str::from_utf8(&b[i + 1..i + 3])
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match h {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn handle(mut s: TcpStream, sh: &Shared) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(3)))?;
    let _ = s.set_nodelay(true);
    let mut buf = Vec::with_capacity(1024);
    let mut tmp = [0u8; 1024];
    loop {
        let n = s.read(&mut tmp)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 16 * 1024 {
            break;
        }
    }
    let req = String::from_utf8_lossy(&buf);
    let first = req.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    if method == "GET" && path == "/api/stream" {
        return stream(s, sh);
    }

    let (status, ctype, body): (&str, &str, String) = if method != "GET" && method != "HEAD" {
        ("405 Method Not Allowed", "text/plain", "GET only\n".into())
    } else {
        match path {
            "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", PANEL.to_string()),
            "/api/state" => ("200 OK", "application/json", sh.state_json()),
            "/api/cmd" => {
                let mut ok = true;
                for pair in query.split('&').filter(|p| !p.is_empty()) {
                    if let Some(("c", v)) = pair.split_once('=') {
                        ok &= sh.command(&pct_decode(v));
                    }
                }
                if ok {
                    ("200 OK", "application/json", sh.state_json())
                } else {
                    ("400 Bad Request", "application/json", sh.state_json())
                }
            }
            "/api/play" => {
                let n = query
                    .split('&')
                    .find_map(|p| p.strip_prefix("n="))
                    .and_then(|v| v.parse::<i32>().ok());
                match n {
                    Some(n) => {
                        sh.play(n);
                        ("204 No Content", "text/plain", String::new())
                    }
                    None => ("400 Bad Request", "text/plain", "want ?n=<midi>\n".into()),
                }
            }
            "/favicon.ico" => ("204 No Content", "text/plain", String::new()),
            _ => ("404 Not Found", "text/plain", "not found\n".into()),
        }
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes())?;
    if method != "HEAD" {
        s.write_all(body.as_bytes())?;
    }
    s.flush()
}

/// Push the state to the browser as server-sent events until it goes away.
fn stream(mut s: TcpStream, sh: &Shared) -> std::io::Result<()> {
    s.set_nodelay(true)?;
    s.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\n\r\nretry: 1000\n\n",
    )?;
    loop {
        let msg = format!("data: {}\n\n", sh.state_json());
        s.write_all(msg.as_bytes())?;
        s.flush()?;
        if sh.quitting.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(80));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decode() {
        assert_eq!(super::pct_decode("arp%3Doff"), "arp=off");
        assert_eq!(super::pct_decode("a+b%2"), "a b%2");
    }
}

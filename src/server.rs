//! Embedded HTTP server for the live web viewer.
//!
//! When started, binds `127.0.0.1:7070` in a background thread and serves:
//! - `GET /`        — the self-contained HTML viewer page (see `viewer.html`)
//! - `GET /state`   — JSON snapshot of the current world state
//! - `GET /terrain` — JSON of the static terrain grid (sent once by the browser)
//!
//! The server holds an `Arc<RwLock<World>>` so it can read state between ticks
//! without blocking the simulation. The main loop holds the write lock only
//! during `world.step()`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, RwLock};
use std::thread;

use crate::world::World;

/// The HTML viewer, compiled into the binary at build time.
const VIEWER_HTML: &str = include_str!("viewer.html");

/// Spawn the HTTP server on a background thread.
/// Returns the port the server is listening on (always 7070).
pub fn spawn(world: Arc<RwLock<World>>) -> u16 {
    let port = 7070u16;
    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))
        .expect("Failed to bind viewer port 7070");
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let world = Arc::clone(&world);
            thread::spawn(move || {
                handle(&world, stream);
            });
        }
    });
    port
}

fn handle(world: &Arc<RwLock<World>>, mut stream: TcpStream) {
    // Read request (up to 4 KB — enough for any simple GET).
    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return,
    };
    let request = std::str::from_utf8(&buf[..n]).unwrap_or("");
    // Extract method + path from the first line.
    let first_line = request.lines().next().unwrap_or("");
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path   = parts.next().unwrap_or("/");

    if method != "GET" {
        let _ = stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n");
        return;
    }

    if path.starts_with("/restart") {
        let mut seed = None;
        if let Some(pos) = path.find('?') {
            let query = &path[pos+1..];
            for pair in query.split('&') {
                let mut parts = pair.split('=');
                if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                    if k == "seed" {
                        if let Ok(s) = v.parse::<u64>() {
                            seed = Some(s);
                        }
                    }
                }
            }
        }

        let mut w = world.write().unwrap();
        let mut new_cfg = w.cfg.clone();
        if let Some(s) = seed {
            new_cfg.seed = s;
        } else {
            new_cfg.seed = (w.rng.next_u64() % 999999) + 1;
        }
        *w = World::new(new_cfg);

        serve_static(&mut stream, "application/json", b"{\"status\":\"ok\"}");
        return;
    }

    match path {
        "/" | "/index.html" => {
            serve_static(&mut stream, "text/html; charset=utf-8", VIEWER_HTML.as_bytes());
        }
        "/state" => {
            let json = world.read().map(|w| w.to_json()).unwrap_or_default();
            serve_static(&mut stream, "application/json", json.as_bytes());
        }
        "/terrain" => {
            let json = world.read().map(|w| w.terrain_json()).unwrap_or_default();
            serve_static(&mut stream, "application/json", json.as_bytes());
        }
        _ => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nNot Found");
        }
    }
}

fn serve_static(stream: &mut TcpStream, content_type: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Cache-Control: no-cache\r\n\
         \r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
}

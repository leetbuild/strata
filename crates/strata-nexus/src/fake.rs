//! A local stand-in for the Nexus API and its download servers, for tests (this crate's and
//! the CLI's end-to-end ones): plain HTTP on 127.0.0.1, fixed replies by path, `Range`
//! honoured on files. It records each request line with its `apikey` header.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// A running fake server; it lives until the process ends.
pub struct FakeServer {
    /// `http://127.0.0.1:<port>`.
    pub base: String,
    /// `"GET /v1/... apikey=<key>"` for each request, in order.
    pub requests: Arc<Mutex<Vec<String>>>,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl FakeServer {
    /// Adds replies: each path (without the query) answers with its body.
    pub fn add(&self, routes: HashMap<String, Vec<u8>>) {
        if let Ok(mut r) = self.routes.lock() {
            r.extend(routes);
        }
    }

    /// The API base for [`crate::api::Client::with_base`].
    pub fn api(&self) -> String {
        format!("{}/v1", self.base)
    }
}

/// Starts a server with no replies yet ([`FakeServer::add`]): JSON for API paths, bytes for
/// files. Unknown paths get 404 with a Nexus-style message.
pub fn serve() -> FakeServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let base = format!("http://{}", listener.local_addr().expect("local address"));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let routes: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
    let table = routes.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let Ok(clone) = stream.try_clone() else { continue };
            let mut reader = BufReader::new(clone);
            let mut first = String::new();
            if reader.read_line(&mut first).unwrap_or(0) == 0 {
                continue;
            }
            let (mut range, mut key, mut length) = (None, String::new(), 0usize);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("range: bytes=") {
                    range = v.trim().trim_end_matches('-').parse::<usize>().ok();
                } else if lower.starts_with("apikey:") {
                    key = line[7..].trim().to_owned();
                } else if let Some(v) = lower.strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; length];
            let _ = std::io::Read::read_exact(&mut reader, &mut body);
            let mut parts = first.split_whitespace();
            let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            seen.lock().map(|mut v| v.push(format!("{method} {target} apikey={key}"))).ok();
            let path = decode(target.split('?').next().unwrap_or(""));
            let mut out = stream;
            let found = table.lock().ok().and_then(|t| t.get(&path).cloned());
            let reply = match found {
                Some(data) => {
                    let start = range.unwrap_or(0).min(data.len());
                    let status = if range.is_some() { "206 Partial Content" } else { "200 OK" };
                    let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n", data.len() - start);
                    head.push_str("x-rl-daily-remaining: 2499\r\nx-rl-hourly-remaining: 99\r\n\r\n");
                    let mut r = head.into_bytes();
                    r.extend_from_slice(&data[start..]);
                    r
                }
                None => {
                    let body = br#"{"message":"No Mod Found"}"#;
                    let mut r = format!("HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                    r.extend_from_slice(body);
                    r
                }
            };
            let _ = out.write_all(&reply);
        }
    });
    FakeServer { base, requests, routes }
}

/// The routes of one Nexus file: its info, its mod's info, its download link (pointing back
/// at this server's `/cdn/<file_name>`), the file itself and `users/validate`.
#[allow(clippy::too_many_arguments)]
pub fn nexus_file_routes(
    base: &str,
    game: &str,
    mod_id: i64,
    file_id: i64,
    mod_name: &str,
    file_name: &str,
    version: &str,
    data: Vec<u8>,
) -> HashMap<String, Vec<u8>> {
    let api = format!("/v1/games/{game}/mods/{mod_id}");
    let file = serde_json::json!({
        "file_id": file_id, "name": format!("{mod_name} Main File"), "version": version, "category_id": 1,
        "file_name": file_name, "size_kb": data.len() / 1024, "uploaded_timestamp": 1_700_000_000,
        "mod_version": version, "description": "The main file."
    });
    let module = serde_json::json!({
        "mod_id": mod_id, "name": mod_name, "summary": "s", "description": "d", "version": version,
        "category_id": 42, "author": "someone", "updated_timestamp": 1_700_000_000
    });
    let links = serde_json::json!([{ "name": "Nexus CDN", "short_name": "Nexus CDN", "URI": format!("{base}/cdn/{file_name}") }]);
    HashMap::from([
        (format!("{api}/files/{file_id}"), file.to_string().into_bytes()),
        (api.clone(), module.to_string().into_bytes()),
        (format!("{api}/files/{file_id}/download_link"), links.to_string().into_bytes()),
        (format!("/cdn/{file_name}"), data),
        ("/v1/users/validate".to_owned(), br#"{"user_id":1,"name":"tester","is_premium":false}"#.to_vec()),
    ])
}

/// `%XX` escapes back to bytes (UTF-8).
fn decode(path: &str) -> String {
    let b = path.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = path.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

//! Nexus single sign-on, as MO2 2.5.2's `NexusSSOLogin` (`nxmaccessmanager.cpp`) speaks it.
//!
//! 1. Open a websocket to `wss://sso.nexusmods.com`.
//! 2. Send `{"id": <random UUID>, "protocol": 2}`.
//! 3. The first reply carries `data.connection_token`: open the browser on
//!    `https://www.nexusmods.com/sso?id=<UUID>&application=<slug>`, where the user allows it.
//! 4. The second reply carries `data.api_key`: the user's personal API key.
//!
//! A reply with `success: false` ends it with its `error`. MO2 gives up when the socket has
//! not answered the first message within 10 seconds; waiting for the browser has no limit
//! (the user cancels).
//!
//! The application slug is the one Nexus registered for the program; Strata has none of its
//! own yet and uses MO2's ([`APPLICATION`]), like the other MO2 ports do.

use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::{Error, Result};

/// MO2's SSO server.
pub const SSO_URL: &str = "wss://sso.nexusmods.com";
/// The page the browser opens; `{id}` and `{application}` are filled in.
pub const SSO_PAGE: &str = "https://www.nexusmods.com/sso?id={id}&application={application}";
/// The application slug sent to the SSO page (MO2's).
pub const APPLICATION: &str = "modorganizer2";

/// Where the login stands, for a dialog (`NexusSSOLogin::States`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    ConnectingToSso,
    WaitingForToken,
    /// The browser was opened on this page.
    WaitingForBrowser(String),
    Finished,
}

/// A random version 4 UUID (`boost::uuids::random_generator`), from the kernel's random source.
pub fn new_uuid() -> Result<String> {
    let mut b = [0u8; 16];
    let mut f = std::fs::File::open("/dev/urandom").map_err(|e| Error::Io("/dev/urandom".into(), e))?;
    std::io::Read::read_exact(&mut f, &mut b).map_err(|e| Error::Io("/dev/urandom".into(), e))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]))
}

fn set_timeout(ws: &WebSocket<MaybeTlsStream<TcpStream>>, t: Duration) {
    let stream: &TcpStream = match ws.get_ref() {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::Rustls(s) => s.get_ref(),
        _ => return,
    };
    let _ = stream.set_read_timeout(Some(t));
}

/// The next text message, polling `cancel` every half second; `limit` bounds the wait.
fn next_message(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    cancel: &AtomicBool,
    limit: Option<Duration>,
) -> Result<Value> {
    let start = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = ws.close(None);
            return Err(Error::Cancelled);
        }
        if limit.is_some_and(|l| start.elapsed() > l) {
            return Err(Error::Network("the Nexus login server did not answer in time (timeout)".to_owned()));
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                return serde_json::from_str(text.as_str()).map_err(|e| Error::Format(format!("SSO message: {e}")));
            }
            Ok(Message::Close(_)) => {
                return Err(Error::Network("the Nexus login server closed the connection".to_owned()));
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(Error::Network(format!("Nexus login: {e}"))),
        }
    }
}

/// Logs in through SSO at `server` (normally [`SSO_URL`]) and returns the API key.
/// `page` is the browser page template (normally [`SSO_PAGE`]); `state` hears each step
/// and opens the browser on `WaitingForBrowser`. Setting `cancel` stops the wait.
pub fn login(server: &str, page: &str, state: &mut dyn FnMut(&State), cancel: &AtomicBool) -> Result<String> {
    state(&State::ConnectingToSso);
    let (mut ws, _) = tungstenite::connect(server).map_err(|e| Error::Network(format!("cannot reach {server}: {e}")))?;
    set_timeout(&ws, Duration::from_millis(500));
    state(&State::WaitingForToken);
    let id = new_uuid()?;
    let hello = json!({ "id": id, "protocol": 2 }).to_string();
    ws.send(Message::text(hello)).map_err(|e| Error::Network(format!("Nexus login: {e}")))?;
    let mut limit = Some(Duration::from_secs(10));
    loop {
        let reply = next_message(&mut ws, cancel, limit)?;
        if !reply["success"].as_bool().unwrap_or(false) {
            let _ = ws.close(None);
            let why = reply["error"].as_str().unwrap_or("no reason given");
            return Err(Error::Api { status: 0, message: format!("There was a problem with SSO initialization: {why}") });
        }
        let data = &reply["data"];
        if data.get("connection_token").is_some() {
            let url = page.replace("{id}", &id).replace("{application}", APPLICATION);
            limit = None;
            state(&State::WaitingForBrowser(url));
        } else if let Some(key) = data["api_key"].as_str() {
            let _ = ws.close(None);
            state(&State::Finished);
            return Ok(key.to_owned());
        } else {
            return Err(Error::Format(format!("SSO reply without a token or key: {reply}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A fake SSO server: checks the hello, then answers with the given replies.
    fn serve(replies: Vec<Value>) -> (String, std::thread::JoinHandle<Value>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let hello: Value = match ws.read().unwrap() {
                Message::Text(t) => serde_json::from_str(t.as_str()).unwrap(),
                other => panic!("{other:?}"),
            };
            for r in replies {
                ws.send(Message::text(r.to_string())).unwrap();
            }
            let _ = ws.read();
            hello
        });
        (url, handle)
    }

    #[test]
    fn uuid_shape() {
        let u = new_uuid().unwrap();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(u, new_uuid().unwrap());
    }

    #[test]
    fn login_gets_the_key_after_the_browser() {
        let (url, server) = serve(vec![
            json!({ "success": true, "data": { "connection_token": "tok" }, "error": null }),
            json!({ "success": true, "data": { "api_key": "KEY-123" }, "error": null }),
        ]);
        let mut seen = Vec::new();
        let key = login(&url, SSO_PAGE, &mut |s| seen.push(s.clone()), &AtomicBool::new(false)).unwrap();
        assert_eq!(key, "KEY-123");
        let hello = server.join().unwrap();
        assert_eq!(hello["protocol"], 2);
        let id = hello["id"].as_str().unwrap().to_owned();
        assert_eq!(
            seen,
            [
                State::ConnectingToSso,
                State::WaitingForToken,
                State::WaitingForBrowser(format!("https://www.nexusmods.com/sso?id={id}&application=modorganizer2")),
                State::Finished,
            ]
        );
    }

    #[test]
    fn refusal_and_cancel() {
        let (url, _server) = serve(vec![json!({ "success": false, "data": null, "error": "bad id" })]);
        let err = login(&url, SSO_PAGE, &mut |_| {}, &AtomicBool::new(false)).unwrap_err().to_string();
        assert!(err.contains("bad id"), "{err}");

        // the browser is never confirmed: cancel ends the wait
        let (url, _server) = serve(vec![json!({ "success": true, "data": { "connection_token": "t" } })]);
        let cancel = AtomicBool::new(false);
        let err = login(
            &url,
            SSO_PAGE,
            &mut |s| {
                if matches!(s, State::WaitingForBrowser(_)) {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
            &cancel,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Cancelled), "{err}");
    }
}

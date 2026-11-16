use std::collections::{BTreeMap, VecDeque};
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

const POLL: Duration = Duration::from_millis(5);
const TIMEOUT_MILLIS: u64 = 300;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const TOKEN_PREFIX: &str = "/oauth2/token";

// a reply is the same shape the Python side scripts, so one case input drives both
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Reply {
    Answer { status: u16, body: Value },
    Timeout,
    Close,
}

#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) authorization: Option<String>,
}

struct Script {
    tokens: VecDeque<Reply>,
    detection: Option<VecDeque<Reply>>,
}

impl Script {
    fn admit(&mut self, path: &str) -> Reply {
        let queue = if path.starts_with(TOKEN_PREFIX) {
            Some(&mut self.tokens)
        } else {
            self.detection.as_mut()
        };
        queue
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| Reply::Answer {
                status: 500,
                body: json!({}),
            })
    }
}

pub(crate) struct Server {
    pub(crate) base: String,
    pub(crate) port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub(crate) fn start(
        tokens: Vec<Reply>,
        detection: Option<Vec<Reply>>,
    ) -> Result<Self, Box<HelperError>> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).or_config_error()?;
        listener.set_nonblocking(true).or_config_error()?;
        let address = listener.local_addr().or_config_error()?;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let script = Arc::new(Mutex::new(Script {
            tokens: VecDeque::from(tokens),
            detection: detection.map(VecDeque::from),
        }));
        let recorder = Arc::clone(&seen);
        let flag = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .spawn(move || serve(&listener, &script, &recorder, &flag))
            .or_config_error()?;
        Ok(Self {
            base: format!("http://{address}"),
            port: address.port(),
            seen,
            stop,
            thread: Some(thread),
        })
    }

    pub(crate) fn requests(&self) -> Vec<Seen> {
        match self.seen.lock() {
            Ok(seen) => seen.to_vec(),
            Err(_poisoned) => Vec::new(),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

fn serve(
    listener: &TcpListener,
    script: &Mutex<Script>,
    seen: &Mutex<Vec<Seen>>,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _peer)) => handle(stream, script, seen),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::park_timeout(POLL),
            Err(_error) => break,
        }
    }
}

fn handle(stream: TcpStream, script: &Mutex<Script>, seen: &Mutex<Vec<Seen>>) {
    let _timeout = stream.set_read_timeout(Some(READ_TIMEOUT));
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let Some((method, path)) = request_line(&mut reader) else {
        return;
    };
    let headers = read_headers(&mut reader);
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; length];
    let _read = reader.read_exact(&mut body);
    let authorization = headers.get("authorization").cloned();
    if let Ok(mut recorder) = seen.lock() {
        recorder.push(Seen {
            method,
            path: path.clone(),
            authorization,
        });
    }
    let reply = match script.lock() {
        Ok(mut state) => state.admit(&path),
        Err(_poisoned) => Reply::Close,
    };
    respond(stream, reply);
}

fn request_line(reader: &mut BufReader<TcpStream>) -> Option<(String, String)> {
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    Some((method, path))
}

fn read_headers(reader: &mut BufReader<TcpStream>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    headers
}

fn respond(mut stream: TcpStream, reply: Reply) {
    match reply {
        Reply::Close => {
            let _shutdown = stream.shutdown(Shutdown::Both);
        }
        Reply::Timeout => {
            thread::park_timeout(Duration::from_millis(TIMEOUT_MILLIS));
            answer(&mut stream, 200, &json!({"access_token": "late"}));
        }
        Reply::Answer { status, body } => answer(&mut stream, status, &body),
    }
}

fn answer(stream: &mut TcpStream, status: u16, body: &Value) {
    let encoded = serde_json::to_vec(body).unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        encoded.len()
    );
    let _head_written = stream.write_all(head.as_bytes());
    let _body_written = stream.write_all(&encoded);
    let _flushed = stream.flush();
    let _shutdown = stream.shutdown(Shutdown::Write);
}

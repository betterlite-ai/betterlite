use std::collections::{BTreeMap, BTreeSet};
use std::io::{Error as IoError, ErrorKind, Read as _, Result as IoResult, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use super::Reply;
use super::helper_failure;
use betterlite_oracle_helper_api::error::HelperError;

const READ_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY: Duration = Duration::from_millis(10);

// a one-request-at-a-time HTTP server both helpers answer from, so the wire bytes are the compared part
pub(crate) struct Server {
    pub(crate) base: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    pub(crate) fn start(reply: &Reply) -> Result<Self, Box<HelperError>> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(helper_failure)?;
        let address = listener.local_addr().map_err(helper_failure)?;
        let answer = response(reply.status, &reply.body);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let recorded = Arc::clone(&requests);
        let halted = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .spawn(move || serve(&listener, &answer, &recorded, &halted))
            .map_err(helper_failure)?;
        Ok(Self {
            base: format!("http://{address}"),
            requests,
            stop,
            worker: Some(worker),
        })
    }

    pub(crate) fn records(&self, extra: &BTreeSet<String>) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|sent| request_record(sent, extra))
            .collect()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        drop(self.worker.take());
    }
}

fn serve(listener: &TcpListener, answer: &[u8], requests: &Mutex<Vec<String>>, stop: &AtomicBool) {
    if listener.set_nonblocking(true).is_err() {
        return;
    }
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _peer)) => answer_once(stream, answer, requests),
            Err(error) if error.kind() == ErrorKind::WouldBlock => thread::park_timeout(RETRY),
            Err(_other) => return,
        }
    }
}
fn answer_once(mut stream: TcpStream, answer: &[u8], requests: &Mutex<Vec<String>>) {
    let _blocking = stream.set_nonblocking(false);
    let _deadline = stream.set_read_timeout(Some(READ_TIMEOUT));
    if let Ok(received) = read_request(&mut stream) {
        requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(received);
    }
    drop(stream.write_all(answer));
}

fn response(status: u16, body: &Value) -> Vec<u8> {
    let text = body.to_string();
    format!(
        "HTTP/1.1 {status} Fixture\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
        text.len()
    )
    .into_bytes()
}
fn read_request(stream: &mut TcpStream) -> IoResult<String> {
    let mut received = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(IoError::from(ErrorKind::UnexpectedEof));
        }
        received.extend_from_slice(chunk.get(..read).unwrap_or_default());
        let Some(head_end) = received.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let declared = content_length(received.get(..head_end).unwrap_or_default());
        let body_start = head_end.checked_add(4).unwrap_or(head_end);
        if received.len().saturating_sub(body_start) >= declared {
            return String::from_utf8(received).map_err(IoError::other);
        }
    }
}
fn content_length(head: &[u8]) -> usize {
    let Ok(text) = str::from_utf8(head) else {
        return 0;
    };
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(name, value)| {
            if name.eq_ignore_ascii_case("content-length") {
                return value.trim().parse().ok();
            }
            None
        })
        .unwrap_or(0)
}

fn request_record(sent: &str, extra: &BTreeSet<String>) -> Value {
    let (head, body) = sent.split_once("\r\n\r\n").unwrap_or((sent, ""));
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_owned()))
        .filter(|(name, _value)| {
            name == "content-type" || name == "x-goog-api-key" || extra.contains(name)
        })
        .collect();
    let payload = serde_json::from_str::<Value>(body).unwrap_or_default();
    json!({
        "method": method,
        "path": path,
        "query": query,
        "headers": headers,
        "body": payload,
    })
}

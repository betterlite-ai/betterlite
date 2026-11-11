use std::collections::BTreeMap;
use std::io::{ErrorKind, Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use secrecy::SecretString;

use crate::cli_context::CliContext;
use crate::commands::Client;

const ACCEPT_DEADLINE: Duration = Duration::from_secs(5);
const POLL_PAUSE: Duration = Duration::from_millis(5);
const READ_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) reason: &'static str,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) declared_length: Option<usize>,
    pub(crate) pause: Option<Duration>,
}

impl Answer {
    fn new(
        status: u16,
        reason: &'static str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Self {
        Self {
            status,
            reason,
            headers,
            body,
            declared_length: None,
            pause: None,
        }
    }

    pub(crate) fn json(status: u16, body: &str) -> Self {
        Self::new(
            status,
            "OK",
            vec![(
                String::from("content-type"),
                String::from("application/json"),
            )],
            body.as_bytes().to_vec(),
        )
    }

    pub(crate) fn text(status: u16, body: &str) -> Self {
        Self::new(status, "ERROR", Vec::new(), body.as_bytes().to_vec())
    }

    pub(crate) fn empty(status: u16, reason: &'static str) -> Self {
        Self::new(status, reason, Vec::new(), Vec::new())
    }

    pub(crate) fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub(crate) fn declared(mut self, length: usize) -> Self {
        self.declared_length = Some(length);
        self
    }

    pub(crate) fn paused(mut self, pause: Duration) -> Self {
        self.pause = Some(pause);
        self
    }
}

#[derive(Clone)]
pub(crate) struct RecordedRequest {
    pub(crate) request_line: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl RecordedRequest {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(found, _value)| found == name)
            .map(|(_found, value)| value.as_str())
    }

    pub(crate) fn body_text(&self) -> String {
        String::from_utf8(self.body.clone()).unwrap()
    }

    pub(crate) fn path(&self) -> &str {
        self.request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
    }

    pub(crate) fn method(&self) -> &str {
        self.request_line
            .split_whitespace()
            .next()
            .unwrap_or_default()
    }
}

pub(crate) struct TestServer {
    pub(crate) base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    pub(crate) fn start(answers: Vec<Answer>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let handle = thread::Builder::new()
            .spawn(move || serve_answers(&listener, answers, &recorded))
            .unwrap();
        Self {
            base_url: format!("http://{address}"),
            requests,
            handle: Some(handle),
        }
    }

    pub(crate) fn finish(mut self) -> Vec<RecordedRequest> {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _unused = handle.join();
        }
    }
}

fn serve_answers(
    listener: &TcpListener,
    answers: Vec<Answer>,
    recorded: &Arc<Mutex<Vec<RecordedRequest>>>,
) {
    let deadline = Instant::now().checked_add(ACCEPT_DEADLINE).unwrap();
    for answer in answers {
        let Some(mut stream) = accept_before(listener, deadline) else {
            return;
        };
        let request = read_request(&mut stream);
        recorded.lock().unwrap().push(request);
        serve(&mut stream, &answer);
    }
}

fn accept_before(listener: &TcpListener, deadline: Instant) -> Option<TcpStream> {
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _address)) => return Some(stream),
            Err(error) if error.kind() == ErrorKind::WouldBlock => thread::park_timeout(POLL_PAUSE),
            Err(_) => return None,
        }
    }
    None
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    stream.set_read_timeout(Some(READ_TIMEOUT)).unwrap();
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        let read = stream.read(&mut byte).unwrap();
        if read == 0 {
            break;
        }
        head.extend_from_slice(&byte);
    }
    let text = String::from_utf8(head).unwrap();
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    let mut length = 0_usize;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (raw_name, raw_value) = line.split_once(':').unwrap();
        let header_name = raw_name.trim().to_ascii_lowercase();
        let header_value = raw_value.trim().to_owned();
        if header_name == "content-length" {
            length = header_value.parse().unwrap();
        }
        headers.push((header_name, header_value));
    }
    let mut body = vec![0_u8; length];
    if length > 0 {
        stream.read_exact(&mut body).unwrap();
    }
    RecordedRequest {
        request_line,
        headers,
        body,
    }
}

fn serve(stream: &mut TcpStream, answer: &Answer) {
    if let Some(pause) = answer.pause {
        thread::park_timeout(pause);
    }
    let length = answer.declared_length.unwrap_or(answer.body.len());
    let mut head = format!("HTTP/1.1 {} {}\r\n", answer.status, answer.reason);
    for (name, value) in &answer.headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("content-length: ");
    head.push_str(&length.to_string());
    head.push_str("\r\nconnection: close\r\n\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(&answer.body);
    let _wrote_head = stream.write_all(&bytes);
    let _flushed = stream.flush();
    let _shut = stream.shutdown(Shutdown::Write);
}

pub(crate) fn client(base_url: &str, api_key: Option<&str>) -> Client {
    client_with_environment(base_url, api_key, &[])
}

pub(crate) fn client_with_environment(
    base_url: &str,
    api_key: Option<&str>,
    variables: &[(&str, &str)],
) -> Client {
    let context = CliContext {
        base_url: base_url.to_owned(),
        base_url_explicit: true,
        api_key: api_key.map(|key| SecretString::from(key.to_owned())),
        api_key_from_token_file: false,
    };
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Client::new(&context, &environment).expect("the shared client builds")
}

pub(crate) fn client_for(server: &TestServer, api_key: Option<&str>) -> Client {
    client(&server.base_url, api_key)
}

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

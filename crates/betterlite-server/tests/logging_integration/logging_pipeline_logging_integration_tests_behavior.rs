use std::collections::BTreeMap;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_server::server::ServerError;
use betterlite_server::server::chat_setup::chat_executor;
use betterlite_server::server::run_with_listener;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use serde::Deserialize;
use signal_hook::consts::SIGTERM;
use signal_hook::low_level::raise;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

// SIGTERM is process wide and each server installs its own handler, so one server runs per test
static SERIAL: AsyncMutex<()> = AsyncMutex::const_new(());

const SENTINEL: &str = "sk-SENTINEL0123456789abcdefghijklmnop";
const CALL_ID: &str = "call-abc";
const STREAM_CHUNK: &str = concat!(
    r#"data: {"id":"c1","object":"chat.completion.chunk","created":7,"model":"upstream-model","#,
    r#""choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#,
    "\n\n"
);

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the JSON log lines are UTF-8")
    }
}

impl Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Captured {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

// B003's gateway stderr format: one JSON object per line, the service event's fields flattened
#[derive(Debug, Deserialize, Default)]
struct LogRecord {
    #[serde(default)]
    message: String,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    service: Option<String>,
    #[serde(default)]
    call_type: Option<String>,
    #[serde(default)]
    is_error: Option<bool>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    accepted: Option<usize>,
    #[serde(default)]
    completed: Option<usize>,
    #[serde(default)]
    failed: Option<usize>,
    #[serde(default)]
    timed_out: Option<usize>,
    #[serde(default)]
    cancelled: Option<usize>,
    #[serde(default)]
    dropped: Option<usize>,
    #[serde(default)]
    drained: Option<bool>,
}

struct Running {
    address: SocketAddr,
    served: JoinSet<Result<(), ServerError>>,
    captured: Captured,
    _guard: DefaultGuard,
}

impl Running {
    fn log(&self) -> String {
        self.captured.text()
    }

    async fn wait_for_listening(&self) {
        let listening = async {
            while !self.log().contains("betterlite server listening") {
                sleep(Duration::from_millis(10)).await;
            }
        };
        timeout(Duration::from_secs(120), listening)
            .await
            .expect("the server reported its listening port");
    }

    async fn wait_for(&self, wanted: &str) {
        let waited = async {
            while !self.log().contains(wanted) {
                sleep(Duration::from_millis(20)).await;
            }
        };
        timeout(Duration::from_secs(60), waited)
            .await
            .unwrap_or_else(|_| panic!("the log never carried {wanted:?}"));
    }

    async fn shut_down(mut self) -> Vec<LogRecord> {
        raise(SIGTERM).unwrap();
        let joined = timeout(Duration::from_secs(120), self.served.join_next())
            .await
            .expect("the server shut down");
        let served = joined
            .expect("the server task joined")
            .expect("the task ran");
        served.expect("the server served without an error");
        parse(&self.captured.text())
    }
}

async fn start(source: &str) -> Running {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_target(false)
        .with_ansi(false)
        .with_writer(captured.clone())
        .finish();
    let guard = set_default(subscriber);
    let mut environment = BTreeMap::new();
    environment.insert("LITELLM_LOG".to_owned(), "INFO".to_owned());
    environment.insert("JSON_LOGS".to_owned(), "true".to_owned());
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(source)
    .unwrap();
    let chat = chat_executor(&config, &environment, StdRng::seed_from_u64(7)).unwrap();
    let mut served = JoinSet::new();
    served.spawn(run_with_listener(config, chat, listener, "127.0.0.1"));
    Running {
        address,
        served,
        captured,
        _guard: guard,
    }
}

async fn exchange(address: SocketAddr, path: &str, body: &str) -> (u16, String) {
    let head = format!(
        "POST {path} HTTP/1.1\r\nhost: localhost\r\ncontent-type: application/json\r\n\
         authorization: Bearer {SENTINEL}\r\nx-litellm-call-id: {CALL_ID}\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body.as_bytes()).await.unwrap();
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).await.unwrap();
    split_reply(&reply)
}

fn split_reply(reply: &[u8]) -> (u16, String) {
    let text = String::from_utf8(reply.to_vec()).expect("the HTTP reply is UTF-8");
    let (head, body) = text.split_once("\r\n\r\n").expect("an HTTP reply");
    let status_line = head.lines().next().expect("a status line");
    let status = status_line
        .split(' ')
        .nth(1)
        .expect("a status code")
        .parse()
        .expect("a numeric status");
    (status, body.to_owned())
}

fn parse(text: &str) -> Vec<LogRecord> {
    assert!(
        !text.contains(SENTINEL),
        "the sentinel credential reached stderr"
    );
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("every stderr line is JSON"))
        .collect()
}

fn service_events(records: &[LogRecord]) -> Vec<&LogRecord> {
    records
        .iter()
        .filter(|record| record.message == "service event")
        .collect()
}

fn assert_one_call(events: &[&LogRecord], call_type: &str, is_error: bool) {
    assert_eq!(events.len(), 2, "one pre-call and one terminal event");
    let pre_call = events[0];
    assert_eq!(pre_call.call_id.as_deref(), Some(CALL_ID));
    assert_eq!(pre_call.service.as_deref(), Some("proxy_pre_call"));
    assert_eq!(pre_call.call_type.as_deref(), Some(call_type));
    assert_eq!(pre_call.is_error, Some(false));
    let terminal = events[1];
    assert_eq!(terminal.call_id.as_deref(), Some(CALL_ID));
    assert_eq!(terminal.service.as_deref(), Some("self"));
    assert_eq!(terminal.call_type.as_deref(), Some(call_type));
    assert_eq!(terminal.is_error, Some(is_error));
    assert!(
        terminal.duration.is_some(),
        "the terminal event has a duration"
    );
}

fn assert_drained(records: &[LogRecord]) {
    let report = records
        .iter()
        .find(|record| record.message == "logging worker stopped")
        .expect("the shutdown report");
    assert_eq!(report.accepted, Some(2));
    assert_eq!(report.completed, Some(2));
    assert_eq!(report.failed, Some(0));
    assert_eq!(report.drained, Some(true));
}

const MOCK_CHAT: &str = "model_list:\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: sk-SENTINEL0123456789abcdefghijklmnop, mock_response: hello}}\n  - {model_name: embed, litellm_params: {model: openai/embed, api_base: 'http://127.0.0.1:1', mock_response: [0.1, 0.2]}}\n";

#[tokio::test]
async fn a_chat_call_logs_one_pre_call_and_one_terminal_event() {
    let _serial = SERIAL.lock().await;
    let running = start(MOCK_CHAT).await;
    running.wait_for_listening().await;
    let (status, body) = exchange(
        running.address,
        "/v1/chat/completions",
        r#"{"model":"g","messages":[{"role":"user","content":"hi"}]}"#,
    )
    .await;
    assert_eq!(status, 200);
    assert!(body.contains("\"content\":\"hello\""), "{body}");
    let records = running.shut_down().await;
    let events = service_events(&records);
    assert_one_call(&events, "acompletion", false);
    assert_eq!(events[1].error.as_deref(), Some(""));
    assert_drained(&records);
}

#[tokio::test]
async fn an_embeddings_call_records_the_embedding_call_type() {
    let _serial = SERIAL.lock().await;
    let running = start(MOCK_CHAT).await;
    running.wait_for_listening().await;
    let (status, body) = exchange(
        running.address,
        "/v1/embeddings",
        r#"{"model":"embed","input":"hello"}"#,
    )
    .await;
    assert_eq!(status, 200);
    assert!(body.contains("embed"), "{body}");
    let records = running.shut_down().await;
    let events = service_events(&records);
    assert_one_call(&events, "aembedding", false);
    assert_drained(&records);
}

#[tokio::test]
async fn an_upstream_error_logs_one_failed_terminal_event() {
    let _serial = SERIAL.lock().await;
    let source = "model_list:\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: sk-SENTINEL0123456789abcdefghijklmnop}}\n";
    let running = start(source).await;
    running.wait_for_listening().await;
    let (status, _body) = exchange(
        running.address,
        "/v1/chat/completions",
        r#"{"model":"g","messages":[{"role":"user","content":"hi"}]}"#,
    )
    .await;
    assert!(status >= 400, "the failed call answers {status}");
    let records = running.shut_down().await;
    let events = service_events(&records);
    assert_one_call(&events, "acompletion", true);
    let expected = format!("the upstream call failed with HTTP {status}");
    assert_eq!(events[1].error.as_deref(), Some(expected.as_str()));
    assert_drained(&records);
}

#[tokio::test]
async fn a_streamed_chat_call_logs_one_terminal_event() {
    let _serial = SERIAL.lock().await;
    let running = start(MOCK_CHAT).await;
    running.wait_for_listening().await;
    let (status, body) = exchange(
        running.address,
        "/v1/chat/completions",
        r#"{"model":"g","messages":[{"role":"user","content":"hi"}],"stream":true}"#,
    )
    .await;
    assert_eq!(status, 200);
    assert!(body.contains("data: [DONE]"), "{body}");
    let records = running.shut_down().await;
    let events = service_events(&records);
    assert_one_call(&events, "acompletion", false);
    assert_drained(&records);
}

#[tokio::test]
async fn a_cancelled_stream_logs_one_terminal_event() {
    let _serial = SERIAL.lock().await;
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream.local_addr().unwrap();
    let mut origin = JoinSet::new();
    origin.spawn(hold_open(upstream));
    let source = format!(
        "model_list:\n  - {{model_name: g, litellm_params: {{model: openai/up, api_base: 'http://{upstream_address}', api_key: key-1}}}}\n"
    );
    let running = start(&source).await;
    running.wait_for_listening().await;
    let body = r#"{"model":"g","messages":[{"role":"user","content":"hi"}],"stream":true}"#;
    let head = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nhost: localhost\r\ncontent-type: application/json\r\n\
         x-litellm-call-id: {CALL_ID}\r\ncontent-length: {}\r\n\r\n",
        body.len()
    );
    let mut client = TcpStream::connect(running.address).await.unwrap();
    client.write_all(head.as_bytes()).await.unwrap();
    client.write_all(body.as_bytes()).await.unwrap();
    let mut received = Vec::new();
    let mut buffer = [0_u8; 512];
    let read = async {
        while !holds_first_chunk(&received) {
            let count = client.read(&mut buffer).await.unwrap();
            assert!(count > 0, "the stream ended before its first chunk");
            received.extend_from_slice(&buffer[..count]);
        }
    };
    timeout(Duration::from_secs(60), read)
        .await
        .expect("the first streamed chunk arrived");
    drop(client);
    running.wait_for("the request was cancelled").await;
    let records = running.shut_down().await;
    let events = service_events(&records);
    assert_one_call(&events, "acompletion", true);
    assert_eq!(
        events[1].error.as_deref(),
        Some("the request was cancelled")
    );
    assert_drained(&records);
    origin.abort_all();
}

fn holds_first_chunk(bytes: &[u8]) -> bool {
    let needle = b"chat.completion.chunk";
    bytes.windows(needle.len()).any(|window| window == needle)
}

async fn hold_open(listener: TcpListener) {
    let Ok((mut stream, _)) = listener.accept().await else {
        return;
    };
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read_u8().await {
            Ok(byte) => head.push(byte),
            Err(_) => return,
        }
    }
    let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n{STREAM_CHUNK}");
    let _written = stream.write_all(reply.as_bytes()).await;
    sleep(Duration::from_secs(120)).await;
}

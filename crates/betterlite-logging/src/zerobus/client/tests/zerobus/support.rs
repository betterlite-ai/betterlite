use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::zerobus::client::Clock;
use betterlite_logging::zerobus::client::ZerobusIngestClient;
use betterlite_logging::zerobus::client::types::ZerobusConnection;
use betterlite_logging::zerobus::row::{TraceRow, trace_row};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinSet;
use tokio::time::sleep;
use url::form_urlencoded;

pub(crate) const WORKSPACE_ID: &str = "1234567890123456";
pub(crate) const TABLE_NAME: &str = "main.litellm.traces";
pub(crate) const CLIENT_ID: &str = "sp-client-id";
pub(crate) const CLIENT_SECRET: &str = "sp-client-secret";

pub(crate) struct FakeClock(Arc<Mutex<f64>>);

impl FakeClock {
    pub(crate) fn new(now: f64) -> Self {
        Self(Arc::new(Mutex::new(now)))
    }

    pub(crate) fn set(&self, now: f64) {
        *self.0.lock().unwrap() = now;
    }

    pub(crate) fn handle(&self) -> Clock {
        let now = Arc::clone(&self.0);
        Arc::new(move || *now.lock().unwrap())
    }
}

#[derive(Clone)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: String,
    pub(crate) stall: bool,
}

impl Reply {
    pub(crate) fn new(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_owned(),
            stall: false,
        }
    }

    pub(crate) fn stalling() -> Self {
        Self {
            status: 200,
            body: "{}".to_owned(),
            stall: true,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Call {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
}

impl Call {
    pub(crate) fn body_text(&self) -> String {
        String::from_utf8(self.body.clone()).unwrap()
    }

    pub(crate) fn form(&self) -> BTreeMap<String, String> {
        form_urlencoded::parse(&self.body).into_owned().collect()
    }
}

struct Script {
    token: Vec<Reply>,
    insert: Vec<Reply>,
    token_index: usize,
    insert_index: usize,
}

impl Script {
    fn next(&mut self, path: &str) -> Reply {
        if path.ends_with("/oidc/v1/token") {
            take(&self.token, &mut self.token_index)
        } else {
            take(&self.insert, &mut self.insert_index)
        }
    }
}

fn take(replies: &[Reply], index: &mut usize) -> Reply {
    let reply = replies.get(*index).or_else(|| replies.last()).cloned();
    *index = index.saturating_add(1);
    reply.expect("the script answers every request")
}

async fn serve(listener: TcpListener, script: Arc<Mutex<Script>>, calls: Arc<Mutex<Vec<Call>>>) {
    while let Ok((mut stream, _peer)) = listener.accept().await {
        answer(&mut stream, &script, &calls).await;
    }
}

async fn answer(stream: &mut TcpStream, script: &Mutex<Script>, calls: &Mutex<Vec<Call>>) {
    let call = read_request(stream).await;
    let reply = script.lock().unwrap().next(&call.path);
    calls.lock().unwrap().push(call);
    if reply.stall {
        sleep(Duration::from_secs(5)).await;
    }
    write_response(stream, &reply).await;
}

pub(crate) struct FakeZerobus {
    pub(crate) base: String,
    calls: Arc<Mutex<Vec<Call>>>,
    tasks: JoinSet<()>,
}

impl FakeZerobus {
    pub(crate) async fn start(token: Vec<Reply>, insert: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let script = Arc::new(Mutex::new(Script {
            token,
            insert,
            token_index: 0,
            insert_index: 0,
        }));
        let mut tasks = JoinSet::new();
        let recorded = Arc::clone(&calls);
        tasks.spawn(serve(listener, script, recorded));
        Self {
            base: format!("http://{address}"),
            calls,
            tasks,
        }
    }

    pub(crate) fn token_calls(&self) -> Vec<Call> {
        self.calls_ending_with("/oidc/v1/token")
    }

    pub(crate) fn insert_calls(&self) -> Vec<Call> {
        self.calls_ending_with("/insert")
    }

    fn calls_ending_with(&self, suffix: &str) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.path.ends_with(suffix))
            .cloned()
            .collect()
    }
}

impl Drop for FakeZerobus {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

async fn read_request(stream: &mut TcpStream) -> Call {
    let mut head = Vec::new();
    loop {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8(head).unwrap();
    let mut lines = text.lines();
    let request_line = lines.next().unwrap();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap().to_owned();
    let path = parts.next().unwrap().to_owned();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_lowercase(), value.to_owned()))
        .collect();
    let length: usize = headers
        .get("content-length")
        .map_or(0, |value| value.parse().unwrap());
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await.unwrap();
    Call {
        method,
        path,
        headers,
        body,
    }
}

async fn write_response(stream: &mut TcpStream, reply: &Reply) {
    let response = format!(
        "HTTP/1.1 {} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        reply.status,
        reply.body.len(),
        reply.body
    );
    stream.write_all(response.as_bytes()).await.unwrap();
}

pub(crate) fn proxied_client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

pub(crate) fn connection(base: &str) -> ZerobusConnection {
    ZerobusConnection {
        workspace_url: base.to_owned(),
        workspace_id: WORKSPACE_ID.to_owned(),
        server_endpoint: base.to_owned(),
        client_id: CLIENT_ID.to_owned(),
        client_secret: SecretString::from(CLIENT_SECRET),
        table_name: TABLE_NAME.to_owned(),
    }
}

pub(crate) fn client(fake: &FakeZerobus, clock: &FakeClock) -> ZerobusIngestClient {
    client_with(connection(&fake.base), clock, Duration::from_secs(5))
}

pub(crate) fn client_with(
    connection: ZerobusConnection,
    clock: &FakeClock,
    timeout: Duration,
) -> ZerobusIngestClient {
    ZerobusIngestClient {
        connection,
        http: proxied_client(),
        clock: clock.handle(),
        timeout,
        token: AsyncMutex::new(None),
    }
}

pub(crate) async fn closed_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{address}")
}

pub(crate) fn token_reply(token: &str, expires_in: &str) -> String {
    format!(r#"{{"access_token": "{token}", "expires_in": {expires_in}}}"#)
}

pub(crate) fn payload(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn expected(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn row(text: &str) -> TraceRow {
    trace_row(&payload(text)).unwrap()
}

pub(crate) fn row_json(row: &TraceRow) -> IndexMap<String, Json> {
    let encoded = serde_json::to_string(row).unwrap();
    serde_json::from_str(&encoded).unwrap()
}

pub(crate) fn trace_rows(texts: &[&str]) -> Vec<TraceRow> {
    texts.iter().map(|text| row(text)).collect()
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_a2a::version_convert::JsonObject;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::agents::json::AgentJson;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::agent_endpoints::a2a_stream::ports::StreamEventConverter;

// nothing listens on port 1, so a request to it is refused at once
pub(crate) const CLOSED: &str = "http://127.0.0.1:1";

// counts its calls on the event, so a test sees whether an event was converted and how often
#[derive(Clone)]
pub(crate) struct CountingConverter;

impl StreamEventConverter for CountingConverter {
    fn convert(&self, mut event: JsonObject, request_id: Option<&AgentJson>) -> JsonObject {
        let conversions = event
            .get("conversions")
            .and_then(AgentJson::as_u64)
            .unwrap_or_default()
            .saturating_add(1);
        event.insert(String::from("conversions"), AgentJson::from(conversions));
        event.insert(
            String::from("request_id"),
            request_id.cloned().unwrap_or_default(),
        );
        event
    }
}

pub(crate) fn client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::outbound(&policy).unwrap()
}

pub(crate) fn client_with_body_limit(body_bytes: usize) -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let limits = TransportLimits {
        connect_timeout: TransportLimits::DEFAULT.connect_timeout,
        read_timeout: TransportLimits::DEFAULT.read_timeout,
        body_bytes,
        header_bytes: TransportLimits::DEFAULT.header_bytes,
    };
    ProxiedClient::build(base_builder().http1_only(), &policy, limits).unwrap()
}

pub(crate) struct Reply {
    pub(crate) status: u16,
    // extra header lines, each ending in CRLF
    pub(crate) headers: &'static str,
    pub(crate) parts: Vec<&'static [u8]>,
    // keep the connection open after the parts and report when the client closes it
    pub(crate) hold_open: bool,
}

pub(crate) struct Agent {
    pub(crate) url: String,
    pub(crate) requests: Arc<Mutex<Vec<String>>>,
    pub(crate) closed: Arc<Notify>,
    _tasks: JoinSet<()>,
}

struct Connection {
    socket: TcpStream,
    reply: Arc<Reply>,
    requests: Arc<Mutex<Vec<String>>>,
    closed: Arc<Notify>,
}

async fn answer(mut connection: Connection) {
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(connection.socket.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .unwrap()
        .parse()
        .unwrap();
    let mut body = vec![0; length];
    connection.socket.read_exact(&mut body).await.unwrap();
    connection
        .requests
        .lock()
        .unwrap()
        .push(format!("{head}{}", String::from_utf8(body).unwrap()));
    let reply = &connection.reply;
    let status_line = format!(
        "HTTP/1.1 {} Scripted\r\ncontent-type: text/event-stream\r\nconnection: close\r\n{}\r\n",
        reply.status, reply.headers
    );
    connection
        .socket
        .write_all(status_line.as_bytes())
        .await
        .unwrap();
    for part in &reply.parts {
        connection.socket.write_all(part).await.unwrap();
        connection.socket.flush().await.unwrap();
        sleep(Duration::from_millis(20)).await;
    }
    if reply.hold_open {
        let mut byte = [0_u8; 1];
        while matches!(connection.socket.read(&mut byte).await, Ok(1)) {}
        connection.closed.notify_one();
    }
}

pub(crate) async fn serve_agent(reply: Reply) -> Agent {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let closed = Arc::new(Notify::new());
    let shared_reply = Arc::new(reply);
    let log = Arc::clone(&requests);
    let signal = Arc::clone(&closed);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let mut connections = JoinSet::new();
        while let Ok((socket, _)) = listener.accept().await {
            connections.spawn(answer(Connection {
                socket,
                reply: Arc::clone(&shared_reply),
                requests: Arc::clone(&log),
                closed: Arc::clone(&signal),
            }));
        }
    });
    Agent {
        url,
        requests,
        closed,
        _tasks: tasks,
    }
}

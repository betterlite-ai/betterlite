#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use http::StatusCode;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::arize::phoenix::client::PhoenixClient;

pub(crate) const PROMPT_BODY: &str = r#"{"data":{"id":"UHJvbXB0VmVyc2lvbjox","description":"answers questions","model_provider":"OPENAI","model_name":"gpt-4o","template":{"type":"chat","messages":[{"role":"system","content":[{"type":"text","text":"You answer questions about {{ topic }}."}]},{"role":"user","content":[{"type":"text","text":"{{ question }}"}]}]},"template_type":"CHAT","template_format":"MUSTACHE","invocation_parameters":{"type":"openai","openai":{"temperature":0.5,"max_tokens":256,"top_p":0.9}}}}"#;

pub(crate) struct FakePhoenix {
    pub(crate) base: String,
    pub(crate) requests: Arc<Mutex<Vec<String>>>,
    _tasks: JoinSet<()>,
}

impl FakePhoenix {
    pub(crate) fn request_heads(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

async fn answer(
    mut stream: TcpStream,
    status: u16,
    body: &'static str,
    requests: Arc<Mutex<Vec<String>>>,
) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let Ok(byte) = stream.read_u8().await else {
            return;
        };
        head.push(byte);
    }
    requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(String::from_utf8(head).unwrap());
    let reason = StatusCode::from_u16(status)
        .unwrap()
        .canonical_reason()
        .unwrap_or_default();
    let reply = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
}

pub(crate) async fn serve(status: u16, body: &'static str) -> FakePhoenix {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let mut connections = JoinSet::new();
        while let Ok((stream, _)) = listener.accept().await {
            connections.spawn(answer(stream, status, body, Arc::clone(&seen)));
        }
    });
    FakePhoenix {
        base,
        requests,
        _tasks: tasks,
    }
}

pub(crate) fn proxied() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::outbound(&policy).unwrap()
}

pub(crate) fn phoenix_client(base: &str) -> PhoenixClient {
    PhoenixClient::new(
        proxied(),
        Some(SecretString::from("secret-key")),
        Some(base.to_owned()),
    )
    .unwrap()
}

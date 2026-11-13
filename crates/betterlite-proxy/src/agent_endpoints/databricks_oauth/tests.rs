#![cfg(test)]

mod cache;
mod config;
mod fetch;
mod header;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::sleep;

use super::config::DatabricksAppOAuthConfig;

// nothing listens on port 1, so a request to it is refused at once
const CLOSED: &str = "http://127.0.0.1:1";

struct Upstream {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    _tasks: JoinSet<()>,
}

fn client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::outbound(&policy).unwrap()
}

fn config(url: &str, secret: &str) -> DatabricksAppOAuthConfig {
    DatabricksAppOAuthConfig {
        client_id: "id".to_owned(),
        client_secret: SecretString::from(secret),
        token_url: format!("{url}/oidc/v1/token"),
        scope: "all-apis sql".to_owned(),
    }
}

async fn serve_one(
    mut socket: TcpStream,
    status: u16,
    body: String,
    delay: Duration,
    requests: Arc<Mutex<Vec<String>>>,
) {
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(socket.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .unwrap()
        .parse()
        .unwrap();
    let mut form = vec![0; length];
    socket.read_exact(&mut form).await.unwrap();
    requests
        .lock()
        .unwrap()
        .push(format!("{head}{}", String::from_utf8(form).unwrap()));
    sleep(delay).await;
    let reply = format!(
        "HTTP/1.1 {status} Reply\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(reply.as_bytes()).await.unwrap();
}

async fn upstream(status: u16, body: &str, delay: Duration) -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&requests);
    let reply_body = body.to_owned();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let mut connections = JoinSet::new();
        while let Ok((socket, _)) = listener.accept().await {
            connections.spawn(serve_one(
                socket,
                status,
                reply_body.clone(),
                delay,
                Arc::clone(&log),
            ));
        }
    });
    Upstream {
        url,
        requests,
        _tasks: tasks,
    }
}

#![cfg(test)]

use std::fs;
use std::future::Future;
use std::io;
use std::path::Path;
use std::str;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::join;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::{advance, sleep};
use url::Url;

use rstest::rstest;

use super::{
    HF_CONFIG_BODY_LIMIT, HF_CONFIG_TIMEOUT_SECONDS, HfConfigClient, decode_hf_model_config,
};
use crate::fetch_policy::response::FetchResponse;
use crate::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};

fn pinned_timeout() -> Duration {
    Duration::try_from_secs_f64(HF_CONFIG_TIMEOUT_SECONDS).expect("timeout")
}

async fn serve_once(status: u16, body: &'static str) -> (String, JoinSet<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut buffer = [0_u8; 1024];
        loop {
            let count = socket.read(&mut buffer).await.expect("read");
            let answered = buffer
                .get(..count)
                .is_some_and(|read| read.windows(4).any(|window| window == b"\r\n\r\n"));
            if count == 0 || answered {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.expect("write");
        socket.shutdown().await.expect("shutdown");
    });
    (format!("http://{address}"), tasks)
}

#[derive(Default)]
struct SocketClient {
    seen: Mutex<Vec<ModelInfoRequest>>,
}

impl SocketClient {
    fn seen(&self) -> Vec<ModelInfoRequest> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ModelInfoClient for &SocketClient {
    type Error = io::Error;

    fn get(
        &self,
        request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, io::Error>> + Send {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        async move {
            let url = Url::parse(&request.url).map_err(io::Error::other)?;
            let host = url.host_str().ok_or_else(|| io::Error::other("no host"))?;
            let port = url
                .port_or_known_default()
                .ok_or_else(|| io::Error::other("no port"))?;
            let mut socket = TcpStream::connect((host, port)).await?;
            let line = format!(
                "GET {} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n",
                url.path()
            );
            socket.write_all(line.as_bytes()).await?;
            let mut bytes = Vec::new();
            socket.read_to_end(&mut bytes).await?;
            parse_response(&bytes)
        }
    }
}

fn parse_response(bytes: &[u8]) -> io::Result<FetchResponse> {
    let boundary = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| io::Error::other("no header end"))?;
    let head = bytes
        .get(..boundary)
        .and_then(|head| str::from_utf8(head).ok())
        .ok_or_else(|| io::Error::other("headers are not UTF-8"))?;
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| io::Error::other("no status"))?;
    let body = bytes
        .get(boundary.saturating_add(4)..)
        .unwrap_or_default()
        .to_vec();
    Ok(FetchResponse::new(status, Vec::new(), body))
}

struct StaticClient {
    status: u16,
    body: Vec<u8>,
    delay: Option<Duration>,
}

impl ModelInfoClient for &StaticClient {
    type Error = io::Error;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, io::Error>> + Send {
        let status = self.status;
        let body = self.body.clone();
        let waits = self.delay;
        async move {
            if let Some(waited) = waits {
                sleep(waited).await;
            }
            Ok(FetchResponse::new(status, Vec::new(), body))
        }
    }
}

async fn fetch_from_server(status: u16, body: &'static str) -> Option<i64> {
    let (base_url, mut server) = serve_once(status, body).await;
    let client = SocketClient::default();
    let answer = HfConfigClient {
        client: &client,
        base_url,
    }
    .max_position_embeddings("org/model")
    .await;
    assert!(
        server
            .join_next()
            .await
            .is_some_and(|result| result.is_ok())
    );
    answer
}

async fn fetch_from_static(client: &StaticClient) -> Option<i64> {
    HfConfigClient {
        client,
        base_url: "https://huggingface.co".to_owned(),
    }
    .max_position_embeddings("org/model")
    .await
}

async fn successful_fetch_records_the_pinned_url_and_timeout() {
    let (base_url, mut server) = serve_once(200, r#"{"max_position_embeddings": 4096}"#).await;
    let client = SocketClient::default();
    let answer = HfConfigClient {
        client: &client,
        base_url: base_url.clone(),
    }
    .max_position_embeddings("org/model")
    .await;
    assert_eq!(answer, Some(4096));
    assert!(
        server
            .join_next()
            .await
            .is_some_and(|result| result.is_ok())
    );
    let seen = client.seen();
    assert_eq!(seen.len(), 1);
    let request = seen.first().expect("request");
    assert_eq!(
        request.url,
        format!("{base_url}/org/model/raw/main/config.json")
    );
    assert_eq!(request.timeout, pinned_timeout());
}

async fn response_failures_become_none() {
    assert_eq!(
        fetch_from_server(404, r#"{"max_position_embeddings": 1}"#).await,
        None
    );
    assert_eq!(fetch_from_server(200, "not json").await, None);
    assert_eq!(
        fetch_from_server(200, r#"{"max_position_embeddings": "4096"}"#).await,
        None
    );
    assert_eq!(fetch_from_server(200, "[4096]").await, None);
}

async fn transport_failures_and_oversized_bodies_become_none() {
    let unused = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = unused.local_addr().expect("address");
    drop(unused);
    assert_eq!(
        HfConfigClient {
            client: &SocketClient::default(),
            base_url: format!("http://{address}"),
        }
        .max_position_embeddings("org/model")
        .await,
        None
    );
    let oversized = StaticClient {
        status: 200,
        body: vec![b' '; HF_CONFIG_BODY_LIMIT.saturating_add(1)],
        delay: None,
    };
    assert_eq!(fetch_from_static(&oversized).await, None);

    let mut padded = br#"{"max_position_embeddings": 4096}"#.to_vec();
    padded.resize(HF_CONFIG_BODY_LIMIT, b' ');
    let at_limit = StaticClient {
        status: 200,
        body: padded,
        delay: None,
    };
    assert_eq!(fetch_from_static(&at_limit).await, Some(4096));
}

#[tokio::test]
async fn fetch_follows_litellm() {
    successful_fetch_records_the_pinned_url_and_timeout().await;
    response_failures_become_none().await;
    transport_failures_and_oversized_bodies_become_none().await;
}

mod fetch_follows_litellm;

#[rstest]
fn replays_the_hf_model_config_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/hf_model_config");
    let mut checked = 0_u32;
    for entry in fs::read_dir(corpus).expect("corpus") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("name")
            .to_owned();
        let bytes = fs::read(&path).expect("read");
        let outcome = decode_hf_model_config(&bytes);
        if name.starts_with("valid-value") || name.starts_with("boundary-value") {
            assert!(matches!(outcome, Ok(Some(_))), "{name}");
        } else if name.starts_with("valid-empty") {
            assert!(matches!(outcome, Ok(None)), "{name}");
        } else if name.starts_with("malformed")
            || name.starts_with("truncated")
            || name.starts_with("boundary-wrong")
        {
            assert!(outcome.is_err(), "{name}");
        } else {
            panic!("the corpus file {name} names no outcome category");
        }
        checked = checked.saturating_add(1);
    }
    assert!(checked > 0, "the corpus has no seeds");
}

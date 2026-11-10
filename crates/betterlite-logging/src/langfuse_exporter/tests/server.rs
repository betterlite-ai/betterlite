use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

pub(super) struct Recorded {
    pub(super) head: String,
    pub(super) body: Vec<u8>,
}

impl Recorded {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().find_map(|line| {
            let (found, value) = line.split_once(": ")?;
            found.eq_ignore_ascii_case(name).then_some(value)
        })
    }

    pub(super) fn sent_headers(&self) -> [Option<&str>; 6] {
        [
            "content-type",
            "x-langfuse-sdk-name",
            "x-langfuse-sdk-version",
            "x-langfuse-ingestion-version",
            "x-langfuse-public-key",
            "authorization",
        ]
        .map(|name| self.header(name))
    }
}

pub(super) struct Scripted {
    pub(super) base: String,
    pub(super) seen: Arc<Mutex<Vec<Recorded>>>,
    _tasks: JoinSet<()>,
}

pub(super) fn reply(status: u16, location: Option<&str>) -> Vec<u8> {
    let target = location.map_or_else(String::new, |value| format!("location: {value}\r\n"));
    format!("HTTP/1.1 {status} X\r\n{target}content-length: 0\r\nconnection: close\r\n\r\n")
        .into_bytes()
}

async fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .ok()
            .filter(|read| *read > 0)?;
        received.extend_from_slice(chunk.get(..read)?);
        let Some(position) = received.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
            continue;
        };
        let head_end = position.checked_add(4)?;
        let head = String::from_utf8(received.get(..head_end)?.to_vec()).ok()?;
        let length: usize = head
            .to_ascii_lowercase()
            .lines()
            .find_map(|line| line.strip_prefix("content-length: ")?.parse().ok())
            .unwrap_or(0);
        if received.len() >= head_end.checked_add(length)? {
            return Some(Recorded {
                head,
                body: received.get(head_end..)?.to_vec(),
            });
        }
    }
}

async fn accept_loop(
    listener: TcpListener,
    seen: Arc<Mutex<Vec<Recorded>>>,
    mut responses: VecDeque<Vec<u8>>,
) {
    while let Ok((mut stream, _)) = listener.accept().await {
        let Some(request) = read_request(&mut stream).await else {
            continue;
        };
        seen.lock().unwrap().push(request);
        if let Some(response) = responses.pop_front() {
            let _written = stream.write_all(&response).await;
        }
    }
}

pub(super) async fn scripted(responses: Vec<Vec<u8>>) -> Scripted {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Arc<Mutex<Vec<Recorded>>> = Arc::default();
    let mut tasks = JoinSet::new();
    tasks.spawn(accept_loop(
        listener,
        Arc::clone(&seen),
        VecDeque::from(responses),
    ));
    Scripted {
        base,
        seen,
        _tasks: tasks,
    }
}

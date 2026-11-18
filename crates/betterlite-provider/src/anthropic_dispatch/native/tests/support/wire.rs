use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct Body {
    pub(crate) model: String,
    pub(crate) prompt: Option<String>,
    pub(crate) max_tokens: Option<i64>,
    pub(crate) max_tokens_to_sample: Option<i64>,
    pub(crate) top_k: Option<i64>,
    pub(crate) stream: Option<bool>,
    pub(crate) tools: Vec<Tool>,
    pub(crate) messages: Vec<Message>,
    #[serde(flatten)]
    pub(crate) extra: IndexMap<String, Json>,
}
#[derive(Deserialize)]
pub(crate) struct Tool {
    pub(crate) name: String,
}
#[derive(Deserialize)]
pub(crate) struct Message {
    pub(crate) content: Vec<Text>,
}
#[derive(Deserialize)]
pub(crate) struct Text {
    pub(crate) text: String,
}

async fn headers(socket: &mut TcpStream, input: &mut Vec<u8>) -> (usize, usize) {
    loop {
        let mut buf = [0; 4096];
        let count = socket.read(&mut buf).await.unwrap();
        assert_ne!(count, 0);
        input.extend_from_slice(&buf[..count]);
        if let Some(end) = input.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let text = str::from_utf8(&input[..end]).unwrap();
            let length = text
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            return (end.checked_add(4).unwrap(), length);
        }
        assert!(input.len() < 0x4000);
    }
}

pub(super) async fn server(listener: TcpListener, answer: &'static str) -> (Body, String) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut input = Vec::new();
    let (start, length) = headers(&mut socket, &mut input).await;
    let end = start.checked_add(length).unwrap();
    while input.len() < end {
        let mut buf = [0; 4096];
        let count = socket.read(&mut buf).await.unwrap();
        assert_ne!(count, 0);
        input.extend_from_slice(&buf[..count]);
    }
    let text = str::from_utf8(&input[..start]).unwrap();
    let key = text
        .lines()
        .find_map(|line| {
            line.strip_prefix("x-api-key:")
                .map(|value| value.trim().to_owned())
        })
        .unwrap();
    let body: Body = serde_json::from_slice(&input[start..end]).unwrap();
    let (content_type, response_body) = if body.stream == Some(true) {
        ("text/event-stream", format!("data: {answer}\n\n"))
    } else {
        ("application/json", answer.to_owned())
    };
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response_body}",
        response_body.len()
    );
    socket.write_all(reply.as_bytes()).await.unwrap();
    (body, key)
}

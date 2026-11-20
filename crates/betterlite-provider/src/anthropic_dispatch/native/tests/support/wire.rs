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

// A request head through its blank line, with the content-length it declares; a GET declares none.
async fn head(socket: &mut TcpStream, input: &mut Vec<u8>) -> (usize, usize) {
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
                .unwrap_or_default();
            return (end.checked_add(4).unwrap(), length);
        }
        assert!(input.len() < 0x4000);
    }
}

async fn read_body(
    socket: &mut TcpStream,
    input: &mut Vec<u8>,
    start: usize,
    length: usize,
) -> usize {
    let end = start.checked_add(length).unwrap();
    while input.len() < end {
        let mut buf = [0; 4096];
        let count = socket.read(&mut buf).await.unwrap();
        assert_ne!(count, 0);
        input.extend_from_slice(&buf[..count]);
    }
    end
}

// A request as the gateway tests read it: any method, any path, every header lowercased.
#[derive(Debug)]
pub(crate) struct Recorded {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Recorded {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(field, _value)| field == name)
            .map(|(_field, value)| value.as_str())
    }
}

// One loopback answer; a plan holds one per connection the proxy is expected to open.
pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) content_type: &'static str,
    pub(crate) body: String,
}

async fn recorded(socket: &mut TcpStream, input: &mut Vec<u8>) -> Recorded {
    let (start, length) = head(socket, input).await;
    let end = read_body(socket, input, start, length).await;
    let text = str::from_utf8(&input[..start.saturating_sub(4)]).unwrap();
    let mut lines = text.lines();
    let request = lines.next().unwrap().split(' ').collect::<Vec<_>>();
    Recorded {
        method: request[0].to_owned(),
        path: request[1].to_owned(),
        headers: lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_owned()))
            .collect(),
        body: input[start..end].to_vec(),
    }
}

// Serves each planned answer on its own connection, in order, and hands back what the proxy asked.
pub(crate) async fn serve(listener: TcpListener, answers: Vec<Answer>) -> Vec<Recorded> {
    let mut requests = Vec::with_capacity(answers.len());
    for answer in answers {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut input = Vec::new();
        requests.push(recorded(&mut socket, &mut input).await);
        let reply = format!(
            "HTTP/1.1 {} {}\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            answer.status,
            reason(answer.status),
            answer.content_type,
            answer.body.len(),
            answer.body
        );
        socket.write_all(reply.as_bytes()).await.unwrap();
    }
    requests
}

const fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown",
    }
}

pub(crate) async fn server(listener: TcpListener, answer: &'static str) -> (Body, String) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut input = Vec::new();
    let (start, length) = head(&mut socket, &mut input).await;
    let end = read_body(&mut socket, &mut input, start, length).await;
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

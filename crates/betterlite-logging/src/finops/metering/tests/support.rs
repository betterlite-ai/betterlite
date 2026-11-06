#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use chrono::{DateTime, TimeZone as _, Utc};
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tracing::subscriber::{DefaultGuard, set_default};

use crate::finops::metering::clock::UtcClock;
use crate::finops::metering::event::{MeteringEvent, UsageTokens};
use crate::finops::metering::transport::{
    HttpMeteringSender, MeteringPost, MeteringResponse, MeteringSender,
};

pub(super) const MODEL: &str = "gpt-4o";
pub(super) const EVENT_TIME: &str = "2030-01-02T03:04:05.000000+00:00";

pub(super) fn variables(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

const fn usage() -> UsageTokens {
    UsageTokens {
        prompt_tokens: 10,
        completion_tokens: 20,
        total_tokens: 30,
    }
}

pub(super) fn event<'event>() -> MeteringEvent<'event> {
    MeteringEvent {
        model: Some(MODEL),
        response_cost: Some(Decimal::new(12, 4)),
        usage: Some(usage()),
        response_id: Some("chatcmpl-response-1"),
        litellm_call_id: Some("call-1"),
        request_user: Some("end-user-1"),
        user_api_key_user_id: Some("key-user-1"),
        user_api_key_team_id: Some("team-1"),
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FixedClock;

impl UtcClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2030, 1, 2, 3, 4, 5).unwrap()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the metering endpoint is unreachable")]
pub(super) struct Unreachable;

pub(super) fn metering_sender(timeout: Duration) -> HttpMeteringSender {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    HttpMeteringSender::new(&policy, timeout).unwrap()
}

#[derive(Clone, Debug)]
pub(super) struct Posted {
    pub(super) url: String,
    pub(super) api_key: String,
    pub(super) content_type: String,
    pub(super) body: Vec<u8>,
}

impl Posted {
    pub(super) fn parse<Body>(&self) -> Body
    where
        Body: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).unwrap()
    }
}

#[derive(Clone, Debug)]
enum Reply {
    Answer { status: u16, body: String },
    Fail,
}

impl Default for Reply {
    fn default() -> Self {
        Self::Answer {
            status: 200,
            body: String::new(),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct FakeSender {
    posted: Arc<Mutex<Vec<Posted>>>,
    reply: Arc<Mutex<Reply>>,
}

impl FakeSender {
    pub(super) fn answering(status: u16, body: &str) -> Self {
        Self {
            posted: Arc::default(),
            reply: Arc::new(Mutex::new(Reply::Answer {
                status,
                body: body.to_owned(),
            })),
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            posted: Arc::default(),
            reply: Arc::new(Mutex::new(Reply::Fail)),
        }
    }

    pub(super) fn posts(&self) -> Vec<Posted> {
        self.posted.lock().unwrap().clone()
    }

    pub(super) fn only_post(&self) -> Posted {
        let posts = self.posts();
        assert_eq!(posts.len(), 1);
        posts.into_iter().next().unwrap()
    }
}

impl MeteringSender for FakeSender {
    type Error = Unreachable;

    fn post(
        &self,
        post: MeteringPost<'_>,
    ) -> impl Future<Output = Result<MeteringResponse, Unreachable>> + Send {
        self.posted.lock().unwrap().push(Posted {
            url: post.url.to_string(),
            api_key: post.bearer.expose_secret().to_owned(),
            content_type: post.content_type.to_owned(),
            body: post.body,
        });
        let reply = self.reply.lock().unwrap().clone();
        match reply {
            Reply::Answer { status, body } => ready(Ok(MeteringResponse { status, body })),
            Reply::Fail => ready(Err(Unreachable)),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("log capture lock"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture_debug() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}

pub(super) struct Received {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Vec<u8>,
}

// The returned future accepts one request and answers it; the test joins it with the call under test.
pub(super) async fn serve_once(
    status: u16,
    reply: &str,
) -> (String, impl Future<Output = Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let response_body = reply.to_owned();
    let served = async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let received = receive(&mut stream).await;
        answer(&mut stream, status, &response_body).await;
        received
    };
    (format!("http://{address}"), served)
}

async fn receive(stream: &mut TcpStream) -> Received {
    let mut buffer = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 2048];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "the client closed before sending a request");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position.saturating_add(4);
        }
    };
    let head = String::from_utf8(buffer[..header_end].to_vec()).unwrap();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap().split(' ');
    let method = request_line.next().unwrap().to_owned();
    let path = request_line.next().unwrap().to_owned();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
        .collect();
    let length = headers
        .get("content-length")
        .map_or(0, |value| value.parse::<usize>().unwrap());
    let mut body = buffer[header_end..].to_vec();
    while body.len() < length {
        let mut chunk = vec![0_u8; length.saturating_sub(body.len())];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "the client closed before sending the body");
        body.extend_from_slice(&chunk[..read]);
    }
    Received {
        method,
        path,
        headers,
        body,
    }
}

async fn answer(stream: &mut TcpStream, status: u16, reply: &str) {
    let response = format!(
        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
        reply.len()
    );
    stream.write_all(response.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();
}

pub(super) fn assert_metering_request(
    received: &Received,
    content_type: &str,
    api_key: &str,
) -> Vec<u8> {
    assert_eq!(received.method, "POST");
    assert_eq!(received.path, "/api/v1/events");
    assert_eq!(
        received.headers.get("authorization"),
        Some(&format!("Bearer {api_key}"))
    );
    assert_eq!(
        received.headers.get("content-type").map(String::as_str),
        Some(content_type)
    );
    received.body.clone()
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use chrono::DateTime;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tracing::subscriber::{DefaultGuard, set_default};

use super::event::{LitellmParams, SuccessEvent};
use super::initialization::{HeliconeLogger, Settings};

pub(crate) const COMPLETION: &str = r#"{"id": "chatcmpl-1", "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "hello"}}], "usage": {"prompt_tokens": 3, "completion_tokens": 5, "total_tokens": 8}}"#;

pub(crate) struct Received {
    pub request_line: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub(crate) fn json(text: &str) -> Json {
    match serde_json::from_str(text) {
        Ok(parsed) => parsed,
        Err(error) => panic!("the test JSON is invalid ({error}): {text}"),
    }
}

pub(crate) fn event(model: &str) -> SuccessEvent {
    SuccessEvent {
        model: model.to_owned(),
        messages: vec![json(r#"{"role": "user", "content": "hi"}"#)],
        response: json(COMPLETION),
        start_time: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        end_time: DateTime::from_timestamp(1_700_000_001, 300_000_000).unwrap(),
        litellm_params: LitellmParams::default(),
    }
}

pub(crate) fn logger(api_base: &str, key: Option<&str>) -> HeliconeLogger {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    let mut environment = BTreeMap::from([("HELICONE_API_BASE".to_owned(), api_base.to_owned())]);
    if let Some(secret) = key {
        environment.insert("HELICONE_API_KEY".to_owned(), secret.to_owned());
    }
    HeliconeLogger {
        settings: Settings::from_environment(&environment),
        client: ProxiedClient::outbound(&Arc::new(policy)).unwrap(),
        timeout: Duration::from_secs(5),
    }
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn text(&self) -> String {
        match String::from_utf8(self.0.lock().unwrap().clone()) {
            Ok(text) => text,
            Err(error) => panic!("the captured output is not UTF-8: {error}"),
        }
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("lock"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn capture_debug() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}

// The returned future accepts one request and answers it; the test joins it with the call under test.
pub(crate) async fn serve_once(
    status: u16,
    reply: &'static str,
) -> (String, impl Future<Output = Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let served = async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(stream.read_u8().await.unwrap());
        }
        let text = String::from_utf8(head).unwrap();
        let mut lines = text.lines();
        let request_line = lines.next().unwrap().to_owned();
        let headers: BTreeMap<String, String> = lines
            .filter_map(|line| line.split_once(": "))
            .map(|(name, value)| (name.to_lowercase(), value.to_owned()))
            .collect();
        let mut body = vec![0; headers["content-length"].parse().unwrap()];
        stream.read_exact(&mut body).await.unwrap();
        let response = format!(
            "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
            reply.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        Received {
            request_line,
            headers,
            body,
        }
    };
    (base, served)
}

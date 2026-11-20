use std::collections::VecDeque;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};

use crate::rag::gemini_ingestion::port::{
    GeminiReply, GeminiTransport, TransportError, TransportRequest,
};

pub(super) struct Sent {
    pub(super) method: &'static str,
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
    pub(super) timeout: Duration,
}

impl Debug for Sent {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("Sent")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("body_bytes", &self.body.len())
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

pub(super) enum ScriptedReply {
    Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    Failure(String),
}

pub(super) struct Scripted {
    replies: Mutex<VecDeque<ScriptedReply>>,
    sent: Mutex<Vec<Sent>>,
}

impl Scripted {
    pub(super) fn new(replies: Vec<ScriptedReply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn take_sent(&self) -> Vec<Sent> {
        std::mem::take(&mut *self.sent.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn answer(&self) -> Result<GeminiReply, TransportError> {
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match next {
            Some(ScriptedReply::Reply {
                status,
                headers,
                body,
            }) => Ok(GeminiReply {
                status,
                headers: header_map(&headers),
                body,
            }),
            Some(ScriptedReply::Failure(message)) => Err(transport_failure(message)),
            None => Err(transport_failure(
                "the test script has no reply left".to_owned(),
            )),
        }
    }
}

#[async_trait]
impl GeminiTransport for Scripted {
    async fn send(&self, request: TransportRequest) -> Result<GeminiReply, TransportError> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent {
                method: request.method.as_str(),
                url: request.url,
                headers: request.headers,
                body: request.body.into_bytes().unwrap(),
                timeout: request.timeout,
            });
        self.answer()
    }
}

fn transport_failure(message: String) -> TransportError {
    TransportError {
        source: Box::new(std::io::Error::other(message)),
    }
}

fn header_map(pairs: &[(String, String)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    headers
}

pub(super) fn reply(status: u16, headers: &[(&str, &str)], body: &[u8]) -> ScriptedReply {
    ScriptedReply::Reply {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: body.to_vec(),
    }
}

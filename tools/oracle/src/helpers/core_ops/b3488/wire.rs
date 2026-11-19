use std::collections::BTreeSet;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::genai_http::service::{
    ClientSource, GenAiRequest, GenAiTransport, StreamReply, WireReply,
};
use betterlite_core::api::vector_store_http_write::error::{ClientError, header_text};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::arguments::json::Json;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap as HttpHeaders;
use http::header::{HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

use super::{Input, ReplyInput};

// The reply the case declares, read as httpx reads it: the status, the declared headers and the raw body
#[derive(Clone)]
pub(super) struct Reply {
    status: u16,
    headers: HttpHeaders,
    body: Vec<u8>,
}

#[derive(Clone)]
struct Seen {
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

// The client pool the handler asks for its client and the client it hands back, both answering the same case reply
pub(super) struct Source {
    reply: Reply,
    keep: BTreeSet<String>,
    seen: Arc<Mutex<Option<Seen>>>,
}

pub(super) struct Wire {
    reply: Reply,
    keep: BTreeSet<String>,
    seen: Arc<Mutex<Option<Seen>>>,
}

impl Source {
    pub(super) fn new(input: &Input) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            reply: reply_of(&input.reply)?,
            keep: keep_of(input),
            seen: Arc::default(),
        })
    }

    // The request the handler sent, in the shape the Python recorder answers
    pub(super) fn view(&self) -> HelperResult {
        let Some(seen) = self
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        else {
            return Err(missing_request());
        };
        let mut headers = Map::new();
        for (name, value) in &seen.headers {
            headers.insert(name.clone(), Value::String(value.clone()));
        }
        Ok(json!({
            "url": seen.url,
            "method": "POST",
            "headers": Value::Object(headers),
            "body": serde_json::from_slice::<Value>(&seen.body).or_value_error()?,
        }))
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(&self, _ssl_verify: Option<&Json>) -> Result<Wire, ClientError> {
        Ok(Wire {
            reply: self.reply.clone(),
            keep: self.keep.clone(),
            seen: Arc::clone(&self.seen),
        })
    }
}

impl GenAiTransport for Wire {
    fn default_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    // httpx's stream=True keeps the body open, so a streaming request is answered with a stream
    fn send(
        &self,
        request: GenAiRequest,
        streaming: bool,
    ) -> impl Future<Output = Result<WireReply, TransportError>> + Send {
        let seen = Seen {
            url: request.url.to_string(),
            headers: pairs(&request.headers, &self.keep),
            body: request.body,
        };
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(seen);
        ready(Ok(answer(self.reply.clone(), streaming)))
    }
}

// The recorder keeps the provider's own header names and the ones the caller overlaid, all lowercased
fn keep_of(input: &Input) -> BTreeSet<String> {
    let mut keep = BTreeSet::from(["content-type".to_owned(), "x-goog-api-key".to_owned()]);
    if let Some(extra) = &input.extra_headers {
        for name in extra.keys() {
            keep.insert(name.to_lowercase());
        }
    }
    keep
}

fn reply_of(input: &ReplyInput) -> Result<Reply, Box<HelperError>> {
    let mut headers = HttpHeaders::new();
    for (name, value) in &input.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).or_value_error()?;
        let header_value = HeaderValue::from_str(value).or_value_error()?;
        headers.insert(header_name, header_value);
    }
    Ok(Reply {
        status: input.status,
        headers,
        body: input.body.as_bytes().to_vec(),
    })
}

fn pairs(headers: &HttpHeaders, keep: &BTreeSet<String>) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| keep.contains(name.as_str()))
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect()
}

fn answer(reply: Reply, streaming: bool) -> WireReply {
    if streaming {
        return WireReply::Streaming(StreamReply {
            headers: reply.headers,
            body: stream::once(ready(Ok::<Vec<u8>, HttpError>(reply.body))).boxed(),
        });
    }
    WireReply::Buffered(UpstreamResponse {
        status: reply.status,
        headers: reply.headers,
        body: reply.body,
    })
}

fn missing_request() -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: Some(None),
        param: None,
        code: None,
        message: "the transport was never called".to_owned(),
    })
}

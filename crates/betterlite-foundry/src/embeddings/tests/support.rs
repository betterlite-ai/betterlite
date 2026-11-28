use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_azure::image_transport::{ImageReply, ImageRequest, ImageTransport};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::embedding::EmbeddingRecord;
use betterlite_prompts::arguments::json::Object;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::{Builder, Runtime};

use crate::embeddings::call::{
    EmbeddingClientFactory, FoundryEmbeddingRequest, TextEmbedding, TextEmbeddingCall, embedding,
    projected_headers,
};
use crate::embeddings::error::FoundryEmbeddingError;
use crate::embeddings::reply::{
    EmbeddingReply, FoundryEmbeddingResponse, decode_foundry_embedding,
};
use crate::embeddings::request::FoundryEmbeddingOptions;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::client_kind::SuppliedClient;

pub(super) const BASE: &str = "https://acct.services.ai.azure.com";
pub(super) const MODEL: &str = "offer-cohere-embed-english-paygo";
pub(super) const IMAGE_URI: &str = "data:image/png;base64,AAAA";
pub(super) const IMAGE_ONE: &str = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.1,0.2]}],"model":"image-model","usage":{"prompt_tokens":2,"total_tokens":2}}"#;
pub(super) const TEXT_ONE: &str = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.3,0.4]}],"model":"text-model","usage":{"prompt_tokens":3,"total_tokens":3}}"#;
pub(super) const TEXT_EMPTY: &str = r#"{"object":"list","data":[],"model":"text-model","usage":{"prompt_tokens":0,"total_tokens":0}}"#;

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

pub(super) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn key(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn header_map(headers: &[(String, String)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        if let (Ok(header_name), Ok(header_value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            map.append(header_name, header_value);
        }
    }
    map
}

pub(super) fn pairs(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[derive(Clone)]
pub(super) struct Answer {
    pub(super) status: u16,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) failed: bool,
}

impl Answer {
    pub(super) fn json(body: &str) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
            failed: false,
        }
    }

    pub(super) fn headed(headers: &[(&str, &str)], body: &str) -> Self {
        Self {
            status: 200,
            headers: pairs(headers),
            body: body.as_bytes().to_vec(),
            failed: false,
        }
    }

    pub(super) fn statused(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
            failed: false,
        }
    }

    pub(super) fn failed() -> Self {
        Self {
            failed: true,
            ..Self::statused(0, "")
        }
    }
}

pub(super) type Seen = Arc<Mutex<Vec<ImageRequest>>>;

pub(super) struct ImageClient {
    pub(super) answer: Answer,
    pub(super) seen: Seen,
}

impl ImageTransport for ImageClient {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransportError>> + Send {
        self.seen.lock().unwrap().push(request);
        if self.answer.failed {
            return ready(Err(TransportError::Closed));
        }
        ready(Ok(ImageReply {
            status: self.answer.status,
            headers: header_map(&self.answer.headers),
            body: self.answer.body.clone(),
        }))
    }
}

pub(super) struct Factory {
    pub(super) answer: Answer,
    pub(super) seen: Seen,
    pub(super) sync_calls: Mutex<Vec<Duration>>,
    pub(super) async_calls: Mutex<Vec<Duration>>,
    pub(super) runtime: Runtime,
}

impl Factory {
    pub(super) fn new(answer: Answer) -> Self {
        Self {
            answer,
            seen: Seen::default(),
            sync_calls: Mutex::new(Vec::new()),
            async_calls: Mutex::new(Vec::new()),
            runtime: Builder::new_current_thread().build().unwrap(),
        }
    }

    pub(super) fn requests(&self) -> Vec<ImageRequest> {
        self.seen.lock().unwrap().clone()
    }

    pub(super) fn sync_timeouts(&self) -> Vec<Duration> {
        self.sync_calls.lock().unwrap().clone()
    }

    pub(super) fn async_timeouts(&self) -> Vec<Duration> {
        self.async_calls.lock().unwrap().clone()
    }
}

impl EmbeddingClientFactory for Factory {
    type Client = ImageClient;

    fn sync_client(&self, timeout: Duration) -> Self::Client {
        self.sync_calls.lock().unwrap().push(timeout);
        ImageClient {
            answer: self.answer.clone(),
            seen: Arc::clone(&self.seen),
        }
    }

    fn async_client(&self, timeout: Duration) -> Self::Client {
        self.async_calls.lock().unwrap().push(timeout);
        ImageClient {
            answer: self.answer.clone(),
            seen: Arc::clone(&self.seen),
        }
    }
}

#[derive(Clone)]
pub(super) struct TextSeen {
    pub(super) mode: ClientKind,
    pub(super) model: String,
    pub(super) input: Vec<String>,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) options: FoundryEmbeddingOptions,
    pub(super) timeout: Duration,
}

pub(super) struct Text {
    pub(super) body: Vec<u8>,
    pub(super) headers: Vec<(String, String)>,
    pub(super) seen: Mutex<Vec<TextSeen>>,
}

impl Text {
    pub(super) fn replying(body: &str) -> Self {
        Self {
            body: body.as_bytes().to_vec(),
            headers: Vec::new(),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn headed(headers: &[(&str, &str)], body: &str) -> Self {
        let mut text = Self::replying(body);
        text.headers = pairs(headers);
        text
    }

    pub(super) fn calls(&self) -> Vec<TextSeen> {
        self.seen.lock().unwrap().clone()
    }
}

impl TextEmbedding for Text {
    fn embed(
        &self,
        call: TextEmbeddingCall<'_>,
    ) -> impl Future<Output = Result<EmbeddingReply, FoundryEmbeddingError>> + Send {
        self.seen.lock().unwrap().push(TextSeen {
            mode: call.mode,
            model: call.model.to_owned(),
            input: call.input.to_vec(),
            api_base: call.api_base.map(str::to_owned),
            api_key: call.api_key.map(|secret| secret.expose_secret().to_owned()),
            options: call.options.clone(),
            timeout: call.timeout,
        });
        let mut reply = decode_foundry_embedding(&self.body);
        if let Ok(decoded) = &mut reply {
            decoded.additional_headers = projected_headers(&header_map(&self.headers));
        }
        ready(reply)
    }
}

pub(super) fn request<'call>(
    model: &'call str,
    input: &'call [String],
) -> FoundryEmbeddingRequest<'call> {
    FoundryEmbeddingRequest {
        model,
        input,
        options: FoundryEmbeddingOptions::default(),
        api_base: Some(BASE),
        api_key: None,
        timeout: Duration::from_secs(5),
        mode: ClientKind::Sync,
    }
}

pub(super) fn run(
    request: &FoundryEmbeddingRequest<'_>,
    supplied: Option<SuppliedClient<'_, ImageClient>>,
    factory: &Factory,
    text: &Text,
) -> Result<FoundryEmbeddingResponse, FoundryEmbeddingError> {
    factory
        .runtime
        .block_on(embedding(request, supplied, factory, text))
}

pub(super) fn indices(records: &[EmbeddingRecord]) -> Vec<u32> {
    records.iter().map(|record| record.index).collect()
}

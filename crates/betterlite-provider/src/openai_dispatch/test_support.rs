use std::sync::{Arc, Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::text_completion_transport::logging::{
    CompletionLogger, PostCallRecord, PreCallRecord, StreamDefaults,
};
use betterlite_openai::text_completion_transport::sdk::{
    ClientConstructor, ClientKind, HttpClientSource, RawCompletion, SdkArguments, SdkError,
    TextCompletionSdk,
};
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;
use futures_util::StreamExt as _;
use http::HeaderMap;
use secrecy::SecretString;

use crate::operation::OperationError;
use crate::prepared::{PreparedRequest, Upstream};

use super::logging::{ChatPostCall, OpenAiLogger, TextPostCall};
use super::transport::{ProviderAnswer, TextClientSource};

// The request a prepared call would have sent, in the shape a fixture or an oracle case can compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub body_limit: Option<usize>,
}

impl RecordedRequest {
    #[must_use]
    pub fn of(prepared: &PreparedRequest) -> Option<Self> {
        let http = match prepared {
            PreparedRequest::Chat(prepared)
            | PreparedRequest::Stream(prepared)
            | PreparedRequest::Embedding(prepared) => &prepared.http,
            _ => return None,
        };
        Some(Self {
            method: http.method.as_str().to_owned(),
            url: http.url.to_string(),
            headers: http
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        value.to_str().map(str::to_owned).unwrap_or_default(),
                    )
                })
                .collect(),
            body: http.body.clone(),
            body_limit: http.body_limit,
        })
    }

    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _value)| header == name)
            .map(|(_header, value)| value.as_str())
    }
}

// The chat and embedding answer a case supplies, with every prepared request it was handed.
pub struct FixtureAnswer {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub sent: Mutex<Vec<RecordedRequest>>,
}

impl FixtureAnswer {
    #[must_use]
    pub fn new(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body,
            sent: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ProviderAnswer for FixtureAnswer {
    async fn answer(&self, prepared: &PreparedRequest) -> Result<Upstream, OperationError> {
        if let Some(record) = RecordedRequest::of(prepared) {
            self.sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(record);
        }
        Ok(Upstream::Buffered(UpstreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
        }))
    }
}

// The text answer: a buffered SDK reply the transport parses exactly as it parses a real one, or the chunks of a streamed one.
#[derive(Debug, Clone)]
pub struct TextReply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub chunks: Option<Vec<Vec<u8>>>,
}

impl TextReply {
    #[must_use]
    pub fn new(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body,
            chunks: None,
        }
    }

    #[must_use]
    pub fn streaming(status: u16, chunks: Vec<Vec<u8>>) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: Vec::new(),
            chunks: Some(chunks),
        }
    }
}

// What the transport asked its constructor and SDK for, kept so a case can compare the composed call.
#[derive(Debug, Clone)]
pub struct TextAttempt {
    pub kind: ClientKind,
    pub base_url: Option<String>,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub timeout: f64,
    pub max_retries: Option<i64>,
    pub http_client: HttpClientSource,
}

#[derive(Clone)]
pub struct FixtureTextSource {
    reply: TextReply,
    built: Arc<Mutex<Vec<TextAttempt>>>,
    sent: Arc<Mutex<Vec<Object>>>,
}

impl FixtureTextSource {
    #[must_use]
    pub fn new(reply: TextReply) -> Self {
        Self {
            reply,
            built: Arc::new(Mutex::new(Vec::new())),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[must_use]
    pub fn attempts(&self) -> Vec<TextAttempt> {
        self.built
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[must_use]
    pub fn bodies(&self) -> Vec<Object> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TextClientSource for FixtureTextSource {
    type Constructor = FixtureTextConstructor;

    fn constructor(&self, _api_key: Option<&SecretString>) -> FixtureTextConstructor {
        FixtureTextConstructor {
            reply: self.reply.clone(),
            built: Arc::clone(&self.built),
            sent: Arc::clone(&self.sent),
        }
    }
}

pub struct FixtureTextConstructor {
    reply: TextReply,
    built: Arc<Mutex<Vec<TextAttempt>>>,
    sent: Arc<Mutex<Vec<Object>>>,
}

impl ClientConstructor for FixtureTextConstructor {
    type Client = FixtureTextSdk;

    fn construct(
        &self,
        arguments: &SdkArguments,
        kind: ClientKind,
    ) -> Result<Option<Self::Client>, SdkError> {
        // the SDK refuses to build without a key, which the transport reports as the missing-key failure
        if arguments.api_key.is_none() {
            return Ok(None);
        }
        self.built
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(TextAttempt {
                kind,
                base_url: arguments.base_url.clone(),
                api_key: arguments.api_key.clone(),
                organization: arguments.organization.clone(),
                timeout: arguments.timeout,
                max_retries: arguments.max_retries,
                http_client: arguments.http_client,
            });
        Ok(Some(FixtureTextSdk {
            reply: self.reply.clone(),
            sent: Arc::clone(&self.sent),
            kind,
        }))
    }
}

#[derive(Clone)]
pub struct FixtureTextSdk {
    reply: TextReply,
    sent: Arc<Mutex<Vec<Object>>>,
    kind: ClientKind,
}

impl TextCompletionSdk for FixtureTextSdk {
    fn kind(&self) -> ClientKind {
        self.kind
    }

    async fn create(&self, data: &Object) -> Result<RawCompletion, SdkError> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(data.clone());
        if let Some(chunks) = &self.reply.chunks {
            let bytes = futures_util::stream::iter(chunks.iter().cloned().map(Ok)).boxed();
            return Ok(RawCompletion::Stream {
                status: self.reply.status,
                headers: self.reply.headers.clone(),
                body: bytes,
            });
        }
        Ok(RawCompletion::Buffered {
            status: self.reply.status,
            headers: self.reply.headers.clone(),
            body: self.reply.body.clone(),
        })
    }
}

// The logging both main.py and the text transport reach, kept as records a case can read back.
#[derive(Default)]
pub struct FixtureLogger {
    pub chat: Vec<ChatPostCall>,
    pub text: Vec<TextPostCall>,
    pub pre_calls: Vec<PreCallRecord>,
    pub post_calls: Vec<PostCallRecord>,
}

impl CompletionLogger for FixtureLogger {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre_calls.push(record);
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.post_calls.push(record);
    }

    fn stream_defaults(&self) -> StreamDefaults {
        StreamDefaults {
            stream_options: None,
            merge_reasoning_in_choices: false,
            is_function_call: false,
            base_hidden_params: JsonObject::new(),
        }
    }
}

impl OpenAiLogger for FixtureLogger {
    fn chat_post_call(&mut self, record: ChatPostCall) {
        self.chat.push(record);
    }

    fn text_post_call(&mut self, record: TextPostCall) {
        self.text.push(record);
    }

    fn completion(&mut self) -> &mut dyn CompletionLogger {
        self
    }
}

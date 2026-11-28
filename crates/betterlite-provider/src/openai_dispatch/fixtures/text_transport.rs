use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_http::client_kind::ClientKind;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::text_completion_transport::sdk::{
    ClientConstructor, HttpClientSource, RawCompletion, SdkArguments, SdkError, TextCompletionSdk,
};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use secrecy::SecretString;

use crate::openai_dispatch::transport::TextClientSource;

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

    fn create(
        &self,
        data: &Object,
    ) -> impl Future<Output = Result<RawCompletion, SdkError>> + Send {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(data.clone());
        if let Some(chunks) = self.reply.chunks.clone() {
            let bytes = stream::iter(chunks.into_iter().map(Ok::<Vec<u8>, SdkError>)).boxed();
            return ready(Ok(RawCompletion::Stream {
                status: self.reply.status,
                headers: self.reply.headers.clone(),
                body: bytes,
            }));
        }
        ready(Ok(RawCompletion::Buffered {
            status: self.reply.status,
            headers: self.reply.headers.clone(),
            body: self.reply.body.clone(),
        }))
    }
}

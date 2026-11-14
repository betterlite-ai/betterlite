#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::{Extra, Json};
use betterlite_tokenizer::count::error::CountError;
use http::{HeaderMap, Method};
use secrecy::SecretString;

use crate::endpoint_target::target::TargetProvider;
use crate::gemini_embedding_call::call::{EmbeddingCall, Execution};
use crate::gemini_embedding_call::error::GeminiEmbeddingCallError;
use crate::gemini_embedding_call::execute::{Clients, Dependencies, gemini_embedding_call};
use crate::gemini_embedding_call::transport::EmbeddingTransport;
use crate::gemini_embedding_records::input::EmbeddingInput;
use crate::gemini_embedding_records::response::EmbeddingReply;
use crate::gemini_embedding_records::usage::TextTokenCounter;
use crate::token_cache::probe::AccessToken;
use crate::vertex_interactions::mint::{MintAccessToken, MintFuture};

// the catalog marks this model uses_embed_content, so Vertex posts to :embedContent
pub(super) const MODEL: &str = "gemini-embedding-2";

#[derive(Clone, Debug)]
pub(super) struct Sent {
    pub method: Method,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub deadline: Duration,
}

type Replies = Arc<Mutex<VecDeque<(u16, Vec<u8>)>>>;

// Answers in order, so a test lists the replies the call is expected to ask for
#[derive(Clone, Default)]
pub(super) struct Script {
    replies: Replies,
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl Script {
    pub(super) fn answering(replies: &[(u16, &str)]) -> Self {
        let bytes: Vec<(u16, &[u8])> = replies
            .iter()
            .map(|(status, body)| (*status, body.as_bytes()))
            .collect();
        Self::answering_bytes(&bytes)
    }

    pub(super) fn answering_bytes(replies: &[(u16, &[u8])]) -> Self {
        let queued: VecDeque<(u16, Vec<u8>)> = replies
            .iter()
            .map(|(status, body)| (*status, (*body).to_vec()))
            .collect();
        Self {
            replies: Arc::new(Mutex::new(queued)),
            sent: Arc::default(),
        }
    }

    pub(super) fn sent(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }
}

impl EmbeddingTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let payload = match request.body {
            RequestBody::Json(bytes) => bytes,
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Vec::new()
            }
        };
        self.sent.lock().unwrap().push(Sent {
            method: request.method,
            url: request.url.to_string(),
            headers: request.headers,
            body: payload,
            deadline: request.deadline,
        });
        let next = self.replies.lock().unwrap().pop_front();
        ready(match next {
            Some((status, body)) => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            }),
            None => Err(TransportError::Closed),
        })
    }
}

pub(super) struct FixedMint;

impl MintAccessToken for FixedMint {
    fn mint<'mint>(
        &'mint self,
        _credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        Box::pin(ready(Ok(AccessToken {
            token: SecretString::from("minted"),
            project: project.unwrap_or("proj").to_owned(),
        })))
    }
}

pub(super) fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

pub(super) fn input(source: &str) -> EmbeddingInput {
    serde_json::from_str(source).unwrap()
}

pub(super) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

// Every optional field is empty; a test builds the call it needs from this one
pub(super) struct Fixture {
    pub input: EmbeddingInput,
    pub optional_params: Extra,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: Option<Duration>,
    pub extra_headers: BTreeMap<String, String>,
}

impl Fixture {
    pub(super) fn new(source: &str) -> Self {
        Self {
            input: input(source),
            optional_params: Extra::new(),
            api_key: None,
            api_base: None,
            timeout: None,
            extra_headers: BTreeMap::new(),
        }
    }

    pub(super) fn call(&self, provider: TargetProvider, execution: Execution) -> EmbeddingCall<'_> {
        EmbeddingCall {
            provider,
            execution,
            model: MODEL,
            input: &self.input,
            optional_params: &self.optional_params,
            api_key: self.api_key.as_ref(),
            api_base: self.api_base.as_deref(),
            credentials: None,
            vertex_project: Some("proj"),
            vertex_location: Some("us-central1"),
            timeout: self.timeout,
            extra_headers: &self.extra_headers,
        }
    }
}

pub(super) struct CharacterCount;

impl TextTokenCounter for CharacterCount {
    fn count_text(&self, _model: &str, text: &str) -> Result<u32, CountError> {
        u32::try_from(text.chars().count()).map_err(|_error| CountError::Overflow)
    }
}

pub(super) async fn run(
    call: &EmbeddingCall<'_>,
    clients: Clients<'_, Script>,
) -> Result<EmbeddingReply, GeminiEmbeddingCallError> {
    let catalog = catalog();
    gemini_embedding_call(
        call,
        &Dependencies {
            catalog: &catalog,
            mint: &FixedMint,
            counter: &CharacterCount,
            clients,
        },
    )
    .await
}

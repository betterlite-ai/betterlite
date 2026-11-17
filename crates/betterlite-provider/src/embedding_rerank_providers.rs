use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::{embedding_host_settings, embedding_provider_settings::Settings};
use betterlite_context::chat_config_selection::cohere::CohereRoute;
use betterlite_core::embedding_rerank_providers::RerankConfig;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_openai::cohere_chat_v1::reply::CohereV1Reply;
use betterlite_openai::cohere_chat_v2::reply::CohereV2Reply;
use betterlite_openai::cohere_embedding::response::CohereEmbeddingResponse;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::infinity_embedding::InfinityEmbeddingResponse;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers, Object as BaseObject};
use betterlite_openai::voyage_embedding::VoyageEmbeddingVariant;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::engine::Engine;
use http::{HeaderMap, Method};
use secrecy::SecretString;
use url::Url;

use crate::operation::{OperationError, OperationRequest, OperationResult, RequestKind, success};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};
use crate::provider::Adapter;

pub mod chat;
mod decode;
pub mod embedding;
pub mod error;
mod request;
pub mod rerank;
mod tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EmbeddingRerank;

// tests build this variant from its bare name; the call's data is Common
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EmbeddingRerankRequest;

#[derive(Debug, Clone)]
pub struct EmbeddingRerankPrepared(Call);

#[derive(Debug, Clone)]
struct Call {
    url: Url,
    headers: HeaderMap,
    body: Vec<u8>,
    timeout: Duration,
    decode: Decode,
}

#[derive(Debug, Clone)]
enum Decode {
    CohereChat(Box<CohereChatDecode>),
    CohereEmbedding(Box<CohereEmbeddingDecode>),
    VoyageEmbedding(VoyageEmbeddingVariant),
    JinaEmbedding(Box<JinaEmbeddingDecode>),
    InfinityEmbedding,
    Rerank(RerankConfig),
}

#[derive(Debug, Clone)]
struct CohereChatDecode {
    version: CohereRoute,
    model: String,
    created: u64,
    id: String,
}

#[derive(Debug, Clone)]
struct CohereEmbeddingDecode {
    model: String,
    api_key: Option<SecretString>,
    request_data: Object,
    input: Vec<String>,
    encoding: Engine,
}

#[derive(Debug, Clone)]
struct JinaEmbeddingDecode {
    api_key: Option<SecretString>,
    request_data: BaseObject,
}

// every family leaf answers with its own typed reply, so each reaches the caller unchanged
#[derive(Debug)]
pub enum EmbeddingRerankResult {
    CohereV1Chat(Box<CohereV1Reply>),
    CohereV2Chat(Box<CohereV2Reply>),
    CohereEmbedding(Box<CohereEmbeddingResponse>),
    VoyageEmbedding(Box<Json>),
    JinaEmbedding(Box<EmbeddingResponse>),
    InfinityEmbedding(Box<InfinityEmbeddingResponse>),
    Rerank(Box<RerankResponse>),
}

// a model string names its family in the slug the provider registry refuses, so the slug picks the leaf
fn family(model: &str) -> Option<(ProviderId, &str)> {
    let (slug, rest) = model.split_once('/')?;
    let provider = slug.parse::<ProviderId>().ok()?;
    matches!(
        provider,
        ProviderId::Cohere
            | ProviderId::CohereChat
            | ProviderId::Voyage
            | ProviderId::JinaAi
            | ProviderId::Infinity
    )
    .then_some((provider, rest))
}

fn unavailable() -> OperationError {
    OperationError::UnavailableAdapter {
        provider: <EmbeddingRerank as Adapter>::NAME.to_owned(),
    }
}

// the process-wide values main.py reads while it prepares a family call, shared by the three forms
pub struct Common<'request> {
    pub provider: ProviderId,
    pub model: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub headers: Option<&'request Headers>,
    pub extra_headers: &'request Headers,
    pub settings: &'request Settings,
    pub host: &'request embedding_host_settings::Settings,
    pub environment: &'request (dyn EnvironmentLookup + Send + Sync),
    pub timeout: Duration,
}

impl Adapter for EmbeddingRerank {
    const NAME: &'static str = "embedding_rerank";

    fn prepare(
        &self,
        operation: &OperationRequest,
        deployment: &Deployment,
        _catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        let Some((provider, model)) = family(&deployment.model) else {
            return Err(unavailable());
        };
        let settings = Settings::default();
        let host = embedding_host_settings::Settings::default();
        let environment = BTreeMap::new();
        let extra_headers = Headers::new();
        let common = Common {
            provider,
            model,
            api_key: deployment.api_key.as_ref(),
            api_base: Some(deployment.api_base.as_str()),
            headers: None,
            extra_headers: &extra_headers,
            settings: &settings,
            host: &host,
            environment: &environment,
            timeout: deployment.timeout,
        };
        let prepared = match operation.kind() {
            RequestKind::Chat(chat) => chat::prepare(&common, &chat.request, false),
            RequestKind::Stream(chat) => chat::prepare(&common, &chat.request, true),
            RequestKind::Embedding(embedding) => embedding::prepare(&common, &embedding.request),
            RequestKind::Other => return Err(unavailable()),
        }
        .map_err(OperationError::from)?;
        Ok(PreparedRequest::EmbeddingRerank(prepared))
    }

    async fn execute(
        &self,
        client: &ProxiedClient,
        prepared: &PreparedRequest,
        _deployment: &Deployment,
    ) -> Result<Upstream, OperationError> {
        let PreparedRequest::EmbeddingRerank(inner) = prepared else {
            return Err(unavailable());
        };
        let call = &inner.0;
        let request = OutboundRequest {
            method: Method::POST,
            url: call.url.clone(),
            query: Vec::new(),
            headers: call.headers.clone(),
            body: RequestBody::Json(call.body.clone()),
            deadline: call.timeout,
        };
        let response = client
            .send_following(request)
            .await
            .map_err(OperationError::from)?;
        let buffered = response
            .buffered(client.limits.body_bytes)
            .await
            .map_err(OperationError::from)?;
        Ok(Upstream::Buffered(buffered))
    }

    fn decode(
        &self,
        prepared: &PreparedRequest,
        upstream: Upstream,
    ) -> Result<OperationResult, OperationError> {
        let PreparedRequest::EmbeddingRerank(inner) = prepared else {
            return Err(unavailable());
        };
        let Upstream::Buffered(buffered) = upstream else {
            return Err(OperationError::UnsupportedOperation {
                provider: Self::NAME.to_owned(),
                operation: String::from("stream"),
            });
        };
        let answer = decode::answer(&inner.0.decode, &success(buffered)?)?;
        Ok(OperationResult::EmbeddingRerank(answer))
    }
}

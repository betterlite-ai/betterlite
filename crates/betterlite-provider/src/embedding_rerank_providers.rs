use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::{embedding_host_settings, embedding_provider_settings};
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

use crate::operation::{
    ChatOperation, EmbeddingOperation, OperationError, OperationRequest, OperationResult,
    RequestKind,
};
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

// the framework's OperationRequest tests build this variant from its bare name, so it stays a unit;
// the call's data is ChatCallInput, EmbeddingCallInput or RerankCallInput
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

// the process-wide values main.py reads while it prepares a family call
struct Globals {
    settings: embedding_provider_settings::Settings,
    host: embedding_host_settings::Settings,
    environment: BTreeMap<String, String>,
    extra_headers: Headers,
}

struct CallContext<'request> {
    provider: ProviderId,
    model: &'request str,
    deployment: &'request Deployment,
    globals: &'request Globals,
}

fn route(
    operation: &OperationRequest,
    context: &CallContext<'_>,
) -> Result<EmbeddingRerankPrepared, OperationError> {
    match operation.kind() {
        RequestKind::Chat(chat) => chat_form(context, chat, false),
        RequestKind::Stream(chat) => chat_form(context, chat, true),
        RequestKind::Embedding(embedding) => embedding_form(context, embedding),
        RequestKind::Other => Err(unavailable()),
    }
}

fn chat_form(
    context: &CallContext<'_>,
    chat: &ChatOperation,
    stream: bool,
) -> Result<EmbeddingRerankPrepared, OperationError> {
    chat::prepare(&chat::ChatCallInput {
        provider: context.provider,
        model: context.model,
        request: &chat.request,
        stream,
        api_key: context.deployment.api_key.as_ref(),
        api_base: Some(context.deployment.api_base.as_str()),
        headers: None,
        extra_headers: &context.globals.extra_headers,
        settings: &context.globals.settings,
        timeout: context.deployment.timeout,
    })
    .map_err(OperationError::from)
}

fn embedding_form(
    context: &CallContext<'_>,
    embedding: &EmbeddingOperation,
) -> Result<EmbeddingRerankPrepared, OperationError> {
    embedding::prepare(&embedding::EmbeddingCallInput {
        provider: context.provider,
        model: context.model,
        request: &embedding.request,
        api_key: context.deployment.api_key.as_ref(),
        api_base: Some(context.deployment.api_base.as_str()),
        headers: None,
        extra_headers: &context.globals.extra_headers,
        settings: &context.globals.settings,
        host: &context.globals.host,
        environment: &context.globals.environment,
        timeout: context.deployment.timeout,
    })
    .map_err(OperationError::from)
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
        let globals = Globals {
            settings: embedding_provider_settings::Settings::default(),
            host: embedding_host_settings::Settings::default(),
            environment: BTreeMap::new(),
            extra_headers: Headers::new(),
        };
        let context = CallContext {
            provider,
            model,
            deployment,
            globals: &globals,
        };
        route(operation, &context).map(PreparedRequest::EmbeddingRerank)
    }

    async fn execute(
        &self,
        client: &ProxiedClient,
        prepared: &PreparedRequest,
        _deployment: &Deployment,
    ) -> Result<Upstream, OperationError> {
        let PreparedRequest::EmbeddingRerank(prepared) = prepared else {
            return Err(unavailable());
        };
        let call = &prepared.0;
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
        let PreparedRequest::EmbeddingRerank(prepared) = prepared else {
            return Err(unavailable());
        };
        let Upstream::Buffered(response) = upstream else {
            return Err(OperationError::UnsupportedOperation {
                provider: Self::NAME.to_owned(),
                operation: String::from("stream"),
            });
        };
        let response = crate::operation::success(response)?;
        let answer = decode::answer(&prepared.0.decode, &response)?;
        Ok(OperationResult::EmbeddingRerank(answer))
    }
}

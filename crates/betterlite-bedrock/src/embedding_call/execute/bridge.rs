use std::sync::{Mutex, PoisonError};

use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::cohere_legacy_embedding::SdkMode;
use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::cohere_legacy_embedding::execute::ExecuteInput;
use betterlite_openai::cohere_legacy_embedding::execute::LegacyEmbeddingTransport;
use betterlite_openai::cohere_legacy_embedding::execute::execute as cohere_execute;
use betterlite_openai::cohere_legacy_embedding::plan::{
    LegacyEmbeddingPlan, PlanInput, plan as cohere_plan,
};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use futures_util::future::BoxFuture;
use http::{HeaderMap, HeaderName, HeaderValue};

use super::{Item, cohere_timeout};
use crate::bedrock_embedding_io::transport::{
    ClientFactory, EmbeddingReply, EmbeddingRequest, EmbeddingTransport, Timeout, TransferError,
};
use crate::credentials::CredentialSource;
use crate::embedding_call::error::EmbeddingCallError;
use crate::embedding_call::hooks::{Event, Input, Recorder, log_event, replay};
use crate::embedding_call::plan::remaining_options;
use crate::embedding_call::sign::{SigningInput, sign_item};
use crate::embedding_call::{Clock, Dependencies, LogContext};

// LiteLLM builds the async cohere client with litellm.LlmProviders.COHERE
const COHERE_CLIENT_PROVIDER: &str = "cohere";

// B1418 hands the transport one request, so the async branch builds the client with the caller timeout and posts through it
pub(super) struct CohereSender<'client, Factory: ClientFactory, Transport> {
    pub(super) supplied: Option<SuppliedClient<'client, Factory::Client>>,
    pub(super) factory: &'client Factory,
    pub(super) fallback: &'client Transport,
    pub(super) asynchronous: bool,
    pub(super) timeout: Option<Timeout>,
    pub(super) headers: &'client Headers,
    pub(super) timed_out: Mutex<Option<TransferError>>,
}

impl<Factory, Transport> CohereSender<'_, Factory, Transport>
where
    Factory: ClientFactory,
{
    pub(super) fn take_timeout(&self) -> Option<TransferError> {
        self.timed_out
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

impl<Factory, Transport> LegacyEmbeddingTransport for CohereSender<'_, Factory, Transport>
where
    Factory: ClientFactory + Sync,
    Factory::Client: LegacyEmbeddingTransport,
    Transport: LegacyEmbeddingTransport,
{
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        match (self.asynchronous, self.supplied.as_ref()) {
            (true, Some(client)) if client.kind == ClientKind::Asynchronous => {
                post(client.transport, request, self.headers, &self.timed_out).await
            }
            (true, _) => {
                let client = self
                    .factory
                    .async_client(COHERE_CLIENT_PROVIDER, self.timeout.clone());
                post(&client, request, self.headers, &self.timed_out).await
            }
            (false, Some(client)) if client.kind == ClientKind::Sync => {
                client.transport.send(request).await
            }
            (false, _) => self.fallback.send(request).await,
        }
    }
}

async fn post<Client>(
    client: &Client,
    request: OutboundRequest,
    headers: &Headers,
    timed_out: &Mutex<Option<TransferError>>,
) -> Result<UpstreamResponse, TransportError>
where
    Client: EmbeddingTransport,
{
    match client.post(embedding_request(request, headers)).await {
        Ok(reply) => Ok(upstream(reply)),
        // the async handler raises litellm.Timeout with status 408, and the recorded cause reaches the caller
        Err(error @ TransferError::Timeout { .. }) => {
            *timed_out.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
            Ok(UpstreamResponse {
                status: 408,
                headers: HeaderMap::new(),
                body: Vec::new(),
            })
        }
        Err(TransferError::Failed(error)) => Err(error),
    }
}

fn embedding_request(request: OutboundRequest, headers: &Headers) -> EmbeddingRequest {
    EmbeddingRequest {
        url: request.url.to_string(),
        headers: headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        body: match request.body {
            RequestBody::Json(bytes) | RequestBody::Bytes { data: bytes, .. } => bytes,
            RequestBody::Empty | RequestBody::Multipart { .. } => Vec::new(),
        },
    }
}

fn upstream(reply: EmbeddingReply) -> UpstreamResponse {
    let headers = reply
        .headers
        .into_iter()
        .filter_map(|(name, value)| {
            Some((
                HeaderName::try_from(name).ok()?,
                HeaderValue::try_from(value).ok()?,
            ))
        })
        .collect();
    UpstreamResponse {
        status: reply.status,
        headers,
        body: reply.body,
    }
}

pub(super) fn cohere<'run, Source, Factory, Transport>(
    item: &'run Item<'_, '_, '_>,
    data: &'run Object,
    dependencies: &'run Dependencies<'_, Source, Factory, Transport>,
    supplied: Option<SuppliedClient<'run, Factory::Client>>,
    log: &'run mut LogContext<'_, '_>,
) -> BoxFuture<'run, Result<LegacyEmbeddingResponse, EmbeddingCallError>>
where
    Source: CredentialSource,
    Factory: ClientFactory + Sync,
    Factory::Client: LegacyEmbeddingTransport,
    Transport: LegacyEmbeddingTransport,
{
    Box::pin(async move {
        let legacy = prepare(item, data, dependencies.clock.as_ref(), log)?;
        let sender = CohereSender {
            supplied,
            factory: dependencies.factory,
            fallback: dependencies.transport,
            asynchronous: item.call.aembedding,
            timeout: item.call.timeout.clone(),
            headers: &legacy.headers,
            timed_out: Mutex::new(None),
        };
        let input = ExecuteInput {
            plan: &legacy,
            input: item.call.input,
            api_key: None,
            encoding: dependencies.encoding,
        };
        let mut recorder = Recorder::default();
        let outcome = cohere_execute(&sender, input, &mut recorder).await;
        replay(recorder, log)?;
        if let Some(source) = sender.take_timeout() {
            return Err(EmbeddingCallError::CohereTimeout { source });
        }
        Ok(outcome?)
    })
}

// LiteLLM logs the prepared request before the handler builds a client and sends it
fn prepare(
    item: &Item<'_, '_, '_>,
    data: &Object,
    clock: &dyn Clock,
    log: &mut LogContext<'_, '_>,
) -> Result<LegacyEmbeddingPlan, EmbeddingCallError> {
    let signed = sign_item(
        &serde_json::to_vec(data)?,
        &SigningInput {
            endpoint_url: &item.plan.endpoint_url,
            credentials: item.plan.credentials.as_ref(),
            region: &item.plan.region,
            extra_headers: item.call.extra_headers.unwrap_or_default(),
            api_key: item.call.api_key,
            configured_bearer: item.call.environment.bearer_token,
            now: clock.now(),
        },
    )
    .map_err(EmbeddingCallError::from)?;
    let legacy = cohere_plan(&PlanInput {
        mode: if item.call.aembedding {
            SdkMode::Async
        } else {
            SdkMode::Sync
        },
        model: &item.plan.model,
        input: item.call.input,
        optional_params: &remaining_options(&item.call.optional_params)?,
        supplied_data: Some(data),
        complete_api_base: Some(&signed.url),
        api_key: None,
        headers: &signed.headers.iter().cloned().collect(),
        timeout: cohere_timeout(item.call)?,
    });
    log_event(
        Event::Pre {
            input: Input::Texts(item.call.input.to_vec()),
            body: legacy.data.clone(),
            headers: item.call.aembedding.then(|| legacy.headers.clone()),
            api_base: item.call.aembedding.then(|| legacy.url.clone()),
        },
        log,
    )?;
    Ok(legacy)
}

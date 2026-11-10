use std::future::Future;
use std::str;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_prompts::arguments::json::Json;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use url::Url;

use super::decode::{DecodeInput, LegacyEmbeddingResponse, decode_cohere_legacy_embedding};
use super::error::CohereLegacyEmbeddingError;
use super::plan::LegacyEmbeddingPlan;
use super::{EmbeddingHooks, PostCall, PreCall, SdkMode};
use crate::cohere_embedding::TextEncoding;
use crate::cohere_streams::cohere_error;

// B173's pooled client in production, a recorded reply in tests and in the oracle helper
pub trait LegacyEmbeddingTransport: Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}
impl LegacyEmbeddingTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        self.send_following(request)
            .await?
            .buffered(self.limits.body_bytes)
            .await
    }
}

pub struct ExecuteInput<'call> {
    pub plan: &'call LegacyEmbeddingPlan,
    pub input: &'call [String],
    pub api_key: Option<&'call SecretString>,
    pub encoding: &'call (dyn TextEncoding + Send + Sync),
}

pub async fn execute<T>(
    transport: &T,
    call: ExecuteInput<'_>,
    hooks: &mut dyn EmbeddingHooks,
) -> Result<LegacyEmbeddingResponse, CohereLegacyEmbeddingError>
where
    T: LegacyEmbeddingTransport,
{
    let asynchronous = call.plan.mode == SdkMode::Async;
    hooks.pre_call(PreCall {
        input: call.input,
        api_key: call.api_key,
        body: &call.plan.data,
        headers: asynchronous.then_some(&call.plan.headers),
        api_base: asynchronous.then_some(call.plan.url.as_str()),
    });
    let request = match outbound(call.plan) {
        Ok(request) => request,
        Err(error) => {
            let logged = Json::Text(error.to_string());
            return Err(fail(&call, hooks, &logged, error));
        }
    };
    let reply = match transport.send(request).await {
        Ok(reply) => reply,
        Err(source) => {
            let logged = Json::Text(source.to_string());
            return Err(fail(
                &call,
                hooks,
                &logged,
                CohereLegacyEmbeddingError::from(source),
            ));
        }
    };
    finish(&reply, &call, hooks)
}

// httpx's raise_for_status rejects every status outside 2xx, a redirect that was not followed included
fn finish(
    reply: &UpstreamResponse,
    call: &ExecuteInput<'_>,
    hooks: &mut dyn EmbeddingHooks,
) -> Result<LegacyEmbeddingResponse, CohereLegacyEmbeddingError> {
    if !(200..300).contains(&reply.status) {
        let message = str::from_utf8(&reply.body).unwrap_or_default().to_owned();
        let error = CohereLegacyEmbeddingError::Status(Box::new(cohere_error(
            reply.status,
            message.clone(),
        )));
        return Err(fail(call, hooks, &Json::Text(message), error));
    }
    decode_cohere_legacy_embedding(
        DecodeInput {
            body: &reply.body,
            model: &call.plan.model,
            api_key: call.api_key,
            request_data: &call.plan.data,
            input: call.input,
            encoding: call.encoding,
        },
        hooks,
    )
}

// only the async handler wraps the send in try/except; the blocking one lets the error through
fn fail(
    call: &ExecuteInput<'_>,
    hooks: &mut dyn EmbeddingHooks,
    original_response: &Json,
    error: CohereLegacyEmbeddingError,
) -> CohereLegacyEmbeddingError {
    if matches!(call.plan.mode, SdkMode::Async) {
        hooks.post_call(PostCall {
            input: call.input,
            api_key: call.api_key,
            body: &call.plan.data,
            original_response,
        });
    }
    error
}
fn outbound(plan: &LegacyEmbeddingPlan) -> Result<OutboundRequest, CohereLegacyEmbeddingError> {
    let mut headers = HeaderMap::new();
    for (name, value) in &plan.headers {
        let key = HeaderName::try_from(name.as_str())
            .map_err(|source| CohereLegacyEmbeddingError::HeaderName(name.clone(), source))?;
        let held = HeaderValue::try_from(value.as_str())
            .map_err(|source| CohereLegacyEmbeddingError::HeaderValue(name.clone(), source))?;
        headers.insert(key, held);
    }
    Ok(OutboundRequest {
        method: Method::POST,
        url: Url::parse(&plan.url)?,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(serde_json::to_vec(&plan.data)?),
        deadline: plan.timeout,
    })
}

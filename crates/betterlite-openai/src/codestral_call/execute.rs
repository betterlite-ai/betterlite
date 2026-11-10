use super::error::CodestralCallError;
use super::log::CallLogger;
use super::plan::{Mode, Plan};
use super::stream::{self, ChunkStream};
use crate::codestral_records::RecordSource;
use crate::codestral_records::record_text;
use crate::codestral_records::reply::{CodestralReply, PostCall, process_response};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use futures_util::StreamExt as _;
use futures_util::stream::{TryStreamExt as _, once};
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;
use serde_json::Value;
use url::Url;
pub struct CodestralCall {
    pub client: ProxiedClient,
}
impl CodestralCall {
    pub async fn complete(
        &self,
        plan: &Plan,
        source: RecordSource,
        log: &mut (dyn CallLogger + Send),
    ) -> Result<CodestralReply, CodestralCallError> {
        let request = outbound(plan, &self.client)?;
        let timed = |failure| CodestralCallError::transport(failure, plan.mode == Mode::Async);
        let limit = self.client.limits.body_bytes;
        let response = self.client.send_following(request).await.map_err(timed)?;
        let upstream = response.buffered(limit).await.map_err(timed)?;
        if !(200..=299).contains(&upstream.status) {
            return Err(stream::native_failure(upstream.status, &upstream.body)?);
        }
        let text = record_text(&upstream.body)?;
        let request_data = Value::Object(plan.body.clone());
        let call = PostCall {
            input: &plan.messages,
            api_key: plan.api_key.expose_secret(),
            original_response: &text,
            request_data: &request_data,
        };
        process_response(upstream.status, call, log, &source).map_err(CodestralCallError::Record)
    }
    pub fn stream(
        &self,
        plan: Plan,
        source: RecordSource,
        log: Box<dyn CallLogger + Send>,
    ) -> Result<ChunkStream, CodestralCallError> {
        let request = outbound(&plan, &self.client)?;
        let chunks = once(stream::make_call(
            self.client.clone(),
            request,
            plan,
            source,
            log,
        ));
        Ok(chunks.try_flatten().boxed())
    }
}
pub(super) fn outbound(
    plan: &Plan,
    client: &ProxiedClient,
) -> Result<OutboundRequest, CodestralCallError> {
    let url = Url::parse(&plan.url).map_err(CodestralCallError::Endpoint)?;
    let mut headers = HeaderMap::new();
    for (name, value) in &plan.headers {
        let header_name = HeaderName::try_from(name.as_str()).map_err(|source| {
            CodestralCallError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value = HeaderValue::try_from(value.as_str()).map_err(|source| {
            CodestralCallError::HeaderValue {
                name: name.clone(),
                source,
            }
        })?;
        headers.append(header_name, header_value);
    }
    let body =
        serde_json::to_vec(&plan.body).map_err(|source| CodestralCallError::Encode { source })?;
    let deadline = plan.timeout.unwrap_or(client.limits.read_timeout);
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(body),
        deadline,
    })
}

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use chrono::{DateTime, Utc};
use http::{HeaderMap, Method};

use super::super::error::{ReplicateError, ReplicateFailure};
use super::super::reply::{
    PostCallLogger, ReplicateReply, ReplyInput, TokenCounter, get_prediction_url,
    transform_response,
};
use super::super::request::{
    ReplicateBody, ReplicateDefaults, RequestContext, RequestInput, get_complete_url,
    transform_request, validate_environment,
};
use super::stream::poll_stream;
use super::transport::{REQUEST_TIMEOUT, ReplicateRequest, ReplicateResponse, ReplicateTransport};
use super::{OutputPolicy, ReplicateCompletion, ReplicatePolling, ReplicateSleeper};
use crate::oci_common::Clock;

pub struct Replicate<'deps> {
    pub transport: Arc<dyn ReplicateTransport>,
    pub sleeper: Arc<dyn ReplicateSleeper>,
    pub prompts: &'deps PromptFactory,
    pub counter: &'deps dyn TokenCounter,
    pub logger: &'deps dyn PostCallLogger,
    pub clock: &'deps dyn Clock,
    pub polling: ReplicatePolling,
    pub policy: OutputPolicy,
}

pub struct ReplicateCall<'request> {
    pub model: &'request str,
    pub messages: &'request mut Vec<Message>,
    pub optional_params: &'request mut Object,
    pub defaults: &'request ReplicateDefaults,
    pub custom_prompts: &'request BTreeMap<String, CustomPromptTemplate>,
    pub api_key: Option<&'request str>,
    pub now: DateTime<Utc>,
}

impl ReplicateCall<'_> {
    async fn build_body(
        &mut self,
        prompts: &PromptFactory,
    ) -> Result<ReplicateBody, ReplicateFailure> {
        transform_request(
            RequestInput {
                model: self.model,
                messages: &mut *self.messages,
                optional_params: &mut *self.optional_params,
                defaults: self.defaults,
                custom_prompts: self.custom_prompts,
            },
            &RequestContext {
                prompts,
                now: self.now,
            },
        )
        .await
    }
}

struct PollRequest<'request> {
    model: &'request str,
    messages: &'request [Message],
    prediction_url: String,
    headers: HeaderMap,
    request_data: &'request ReplicateBody,
    api_key: Option<&'request str>,
    created: Option<i64>,
}

fn is_processing(response: &ReplicateResponse) -> Result<bool, ReplicateFailure> {
    if response.status != 200 {
        return Ok(false);
    }
    let value: Json = serde_json::from_slice(&response.body)
        .map_err(|source| ReplicateFailure::Decode { source })?;
    Ok(matches!(
        value.as_object().and_then(|object| object.get("status")),
        Some(Json::Text(status)) if status == "processing" || status == "starting"
    ))
}

impl Replicate<'_> {
    async fn create_prediction(
        &self,
        model: &str,
        headers: &HeaderMap,
        body: &ReplicateBody,
    ) -> Result<String, ReplicateFailure> {
        let payload =
            serde_json::to_vec(body).map_err(|source| ReplicateFailure::Encode { source })?;
        let response = self
            .transport
            .send(ReplicateRequest {
                method: Method::POST,
                url: get_complete_url(model),
                headers: headers.clone(),
                body: Some(payload),
                timeout: REQUEST_TIMEOUT,
            })
            .await
            .map_err(|source| ReplicateFailure::Transport { source })?;
        get_prediction_url(&response.body, &response.headers)
    }

    async fn await_prediction(
        &self,
        request: &PollRequest<'_>,
    ) -> Result<ReplicateResponse, ReplicateFailure> {
        for retry in 0_i32..self.polling.retries {
            let delay = f64::from(retry).mul_add(2.0, self.polling.retry_delay_seconds);
            self.sleeper.sleep(delay).await;
            let response = self
                .transport
                .send(ReplicateRequest {
                    method: Method::GET,
                    url: request.prediction_url.clone(),
                    headers: request.headers.clone(),
                    body: None,
                    timeout: REQUEST_TIMEOUT,
                })
                .await
                .map_err(|source| ReplicateFailure::Transport { source })?;
            if !is_processing(&response)? {
                return Ok(response);
            }
        }
        Err(ReplicateFailure::Api(Box::new(ReplicateError {
            status_code: 500,
            message: String::from("No response received from Replicate API after max retries"),
            headers: None,
        })))
    }

    async fn poll_reply(
        &self,
        request: PollRequest<'_>,
    ) -> Result<ReplicateReply, ReplicateFailure> {
        let response = self.await_prediction(&request).await?;
        let reply = transform_response(
            &ReplyInput {
                model: request.model,
                body: &response.body,
                headers: &response.headers,
                messages: request.messages,
                request_data: request.request_data,
                api_key: request.api_key,
            },
            self.counter,
            self.logger,
        )
        .await?;
        Ok(ReplicateReply {
            model: reply.model,
            created: request.created,
            text: reply.text,
            usage: reply.usage,
        })
    }

    pub async fn complete(
        &self,
        call: ReplicateCall<'_>,
    ) -> Result<ReplicateCompletion, ReplicateFailure> {
        let mut invocation = call;
        let headers = validate_environment(&HeaderMap::new(), invocation.api_key)?;
        let body = invocation.build_body(self.prompts).await?;
        // the sync path stamps the reply with the clock right before the create POST; the async twin leaves it unset
        let created = match self.policy {
            OutputPolicy::Strict => Some(self.clock.now().timestamp()),
            OutputPolicy::Lenient => None,
        };

        let prediction_url = self
            .create_prediction(invocation.model, &headers, &body)
            .await?;
        if matches!(
            invocation.optional_params.get("stream"),
            Some(Json::Bool(true))
        ) {
            return Ok(ReplicateCompletion::Stream(poll_stream(
                self,
                prediction_url,
                headers,
            )));
        }
        let reply = self
            .poll_reply(PollRequest {
                model: invocation.model,
                messages: invocation.messages,
                prediction_url,
                headers,
                request_data: &body,
                api_key: invocation.api_key,
                created,
            })
            .await?;
        Ok(ReplicateCompletion::Reply(reply))
    }
}

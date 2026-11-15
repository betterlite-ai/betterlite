use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::responses_objects::wire::{
    ResponsesObjectsDecodeError, WireDecodeLimits,
};
use betterlite_openai::api::responses_objects::{ResponsesApiResponse, decode_responses_objects};
use betterlite_openai::response_input::error::ResponseInputError;
use betterlite_openai::response_input::{
    ResponsesInput, ResponsesItem, transform_responses_request,
};
use betterlite_prompts::arguments::json::{Json as PromptJson, Object as PromptObject};
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use serde_json::value::RawValue;
use thiserror::Error;
use url::Url;

use super::executor::{AnthropicContinuation, ContinuationExecutor, ResponsesContinuation};

pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(120);

const DECODE_LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0010_0000,
    max_event_bytes: 0x0010_0000,
    max_items: 0x4000,
};

#[derive(Debug, Error)]
pub enum HttpExecutorError {
    #[error(transparent)]
    Request(#[from] ResponseInputError),
    #[error("the agentic continuation needs an absolute api_base")]
    Url(#[source] url::ParseError),
    #[error("the agentic follow-up request could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("the provider answered the agentic follow-up with status {status}: {body}")]
    Status { status: u16, body: String },
    #[error(transparent)]
    Decode(#[from] ResponsesObjectsDecodeError),
    #[error("the agentic follow-up answer could not be read as text")]
    Utf8(#[source] std::string::FromUtf8Error),
    #[error("the anthropic follow-up is served by the messages transport, not this executor")]
    UnsupportedAnthropic,
}

// the leaf the family composition hands recursive SDK execution to: B173 sends, B507 builds, B508 checks
pub struct ResponsesHttpExecutor {
    pub client: ProxiedClient,
    pub api_base: String,
    pub provider: ProviderId,
    pub headers: HeaderMap,
    pub model_info: Option<PromptObject>,
    pub deadline: Duration,
}

#[async_trait]
impl ContinuationExecutor for ResponsesHttpExecutor {
    type Error = HttpExecutorError;

    async fn responses(&self, call: &ResponsesContinuation) -> Result<Box<RawValue>, Self::Error> {
        let url = format!("{}/responses", self.api_base.trim_end_matches('/'));
        let request = OutboundRequest {
            method: Method::POST,
            url: Url::parse(&url).map_err(HttpExecutorError::Url)?,
            query: Vec::new(),
            headers: self.headers.clone(),
            body: RequestBody::Json(self.request_body(call)?),
            deadline: self.deadline,
        };
        let response = self.client.send_buffered(request).await?;
        if !(200..300).contains(&response.status) {
            return Err(HttpExecutorError::Status {
                status: response.status,
                body: String::from_utf8_lossy(&response.body).into_owned(),
            });
        }
        let _decoded: ResponsesApiResponse =
            decode_responses_objects(&response.body, DECODE_LIMITS)?;
        let text = String::from_utf8(response.body).map_err(HttpExecutorError::Utf8)?;
        RawValue::from_string(text).map_err(HttpExecutorError::Encode)
    }

    async fn anthropic_messages(
        &self,
        _call: &AnthropicContinuation,
    ) -> Result<Box<RawValue>, Self::Error> {
        Err(HttpExecutorError::UnsupportedAnthropic)
    }
}

impl ResponsesHttpExecutor {
    fn request_body(&self, call: &ResponsesContinuation) -> Result<Vec<u8>, HttpExecutorError> {
        let mut options: PromptObject = IndexMap::with_capacity(call.params.len());
        for (key, value) in &call.params {
            options.insert(key.clone(), prompt_json(value));
        }
        let mut items = Vec::with_capacity(call.input.len());
        for item in &call.input {
            let parsed: PromptJson =
                serde_json::from_str(item.0.get()).map_err(HttpExecutorError::Encode)?;
            items.push(ResponsesItem(parsed));
        }
        let body = transform_responses_request(
            &call.model,
            &ResponsesInput::Items(items),
            &options,
            self.provider,
            self.model_info.as_ref(),
        )?;
        serde_json::to_vec(&body).map_err(HttpExecutorError::Encode)
    }
}

fn prompt_json(value: &Json) -> PromptJson {
    match value {
        Json::Null => PromptJson::Null,
        Json::Bool(flag) => PromptJson::Bool(*flag),
        Json::Number(number) => PromptJson::Number(number.clone()),
        Json::Text(text) => PromptJson::Text(text.clone()),
        Json::List(items) => PromptJson::List(items.iter().map(prompt_json).collect()),
        Json::Object(fields) => PromptJson::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), prompt_json(item)))
                .collect(),
        ),
    }
}

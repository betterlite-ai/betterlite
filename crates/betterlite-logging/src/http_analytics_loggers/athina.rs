use std::error::Error as StdError;
use std::str;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use url::Url;

use super::delivery::{DELIVERY_DEADLINE, Delivery, DeliveryError, PostRequest};
use super::event::{self, Event, PayloadError};

mod tests;

pub const ADDITIONAL_KEYS: [&str; 13] = [
    "environment",
    "prompt_slug",
    "customer_id",
    "customer_user_id",
    "session_id",
    "external_reference_id",
    "context",
    "expected_response",
    "user_query",
    "tags",
    "user_feedback",
    "model_options",
    "custom_attributes",
];

const DEFAULT_BASE_URL: &str = "https://log.athina.ai";
const ENDPOINT_PATH: &str = "/api/v1/log/inference";

pub struct AthinaLogger<D> {
    pub delivery: D,
    pub settings: AthinaSettings,
}

pub struct AthinaSettings {
    pub api_key: Option<SecretString>,
    pub endpoint: String,
}

impl AthinaSettings {
    pub fn from_environment<E>(environment: &E) -> Self
    where
        E: EnvironmentLookup,
    {
        let api_key = environment
            .lookup("ATHINA_API_KEY")
            .filter(|value| !value.is_empty())
            .map(SecretString::from);
        let base_url = environment
            .lookup("ATHINA_BASE_URL")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
        Self {
            api_key,
            endpoint: format!("{base_url}{ENDPOINT_PATH}"),
        }
    }
}

impl<D: Delivery> AthinaLogger<D> {
    pub async fn log_event(&self, event: &Event<'_>) {
        if let Err(error) = self.deliver(event).await {
            tracing::debug!(?error, "Athina logger error");
        }
    }

    async fn deliver(&self, event: &Event<'_>) -> Result<(), AthinaFailure<D::Error>> {
        let kwargs = event::kwargs_object(event.kwargs)?;
        let Some(response) = selected_response(kwargs, event) else {
            return Ok(());
        };
        let payload = payload(kwargs, &response, &event.start_time, &event.end_time)?;
        let body = serde_json::to_vec(&payload).map_err(DeliveryError::Encode)?;
        let sent = self
            .delivery
            .post(self.request(body)?)
            .await
            .map_err(DeliveryError::Transport)?;
        // LiteLLM logs the response text for any status and only a 200 counts as success
        let text = str::from_utf8(&sent.body).unwrap_or_default();
        if sent.status == 200 {
            tracing::debug!(status = sent.status, text, "Athina logger succeeded");
        } else {
            tracing::debug!(status = sent.status, text, "Athina logger error");
        }
        Ok(())
    }

    fn request(&self, body: Vec<u8>) -> Result<PostRequest, DeliveryError<D::Error>> {
        let Some(api_key) = &self.settings.api_key else {
            return Err(DeliveryError::MissingSetting {
                name: "ATHINA_API_KEY",
            });
        };
        let url = Url::parse(&self.settings.endpoint).map_err(|source| DeliveryError::Url {
            name: "ATHINA_BASE_URL",
            source,
        })?;
        let mut value = HeaderValue::try_from(api_key.expose_secret()).map_err(|source| {
            DeliveryError::Header {
                name: "athina-api-key",
                source,
            }
        })?;
        value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(HeaderName::from_static("athina-api-key"), value);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(PostRequest {
            url,
            headers,
            body,
            deadline: DELIVERY_DEADLINE,
        })
    }
}

fn selected_response(kwargs: &IndexMap<String, Json>, event: &Event<'_>) -> Option<Json> {
    if matches!(kwargs.get("stream"), Some(Json::Bool(true))) {
        kwargs
            .get("complete_streaming_response")
            .map(|buffered| event::response_dump(Some(buffered)))
    } else {
        Some(event::response_dump(event.response))
    }
}

#[derive(Serialize)]
struct AthinaPayload<'call> {
    language_model_id: Json,
    request: &'call IndexMap<String, Json>,
    response: &'call Json,
    prompt_tokens: Json,
    completion_tokens: Json,
    total_tokens: Json,
    response_time: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    functions: Option<&'call Json>,
    #[serde(flatten)]
    additional: AthinaAdditional<'call>,
}

#[derive(Serialize)]
struct AthinaAdditional<'call> {
    #[serde(skip_serializing_if = "Option::is_none")]
    environment: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_slug: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    customer_id: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    customer_user_id: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    external_reference_id: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_response: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_query: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_feedback: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_options: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_attributes: Option<&'call Json>,
}

fn optional_fields(kwargs: &IndexMap<String, Json>) -> (Option<&Json>, Option<&Json>) {
    match kwargs.get("optional_params") {
        Some(Json::Object(optional)) => (optional.get("tools"), optional.get("functions")),
        _ => (None, None),
    }
}

fn additional_fields(
    kwargs: &IndexMap<String, Json>,
) -> Result<AthinaAdditional<'_>, PayloadError> {
    let metadata = event::metadata(kwargs)?;
    Ok(AthinaAdditional {
        environment: metadata.and_then(|fields| fields.get("environment")),
        prompt_slug: metadata.and_then(|fields| fields.get("prompt_slug")),
        customer_id: metadata.and_then(|fields| fields.get("customer_id")),
        customer_user_id: metadata.and_then(|fields| fields.get("customer_user_id")),
        session_id: metadata.and_then(|fields| fields.get("session_id")),
        external_reference_id: metadata.and_then(|fields| fields.get("external_reference_id")),
        context: metadata.and_then(|fields| fields.get("context")),
        expected_response: metadata.and_then(|fields| fields.get("expected_response")),
        user_query: metadata.and_then(|fields| fields.get("user_query")),
        tags: metadata.and_then(|fields| fields.get("tags")),
        user_feedback: metadata.and_then(|fields| fields.get("user_feedback")),
        model_options: metadata.and_then(|fields| fields.get("model_options")),
        custom_attributes: metadata.and_then(|fields| fields.get("custom_attributes")),
    })
}

fn payload<'call>(
    kwargs: &'call IndexMap<String, Json>,
    response: &'call Json,
    start: &DateTime<Utc>,
    end: &DateTime<Utc>,
) -> Result<AthinaPayload<'call>, PayloadError> {
    let (tools, functions) = optional_fields(kwargs);
    Ok(AthinaPayload {
        language_model_id: kwargs.get("model").cloned().unwrap_or(Json::Null),
        request: kwargs,
        response,
        prompt_tokens: event::usage_token(response, "prompt_tokens"),
        completion_tokens: event::usage_token(response, "completion_tokens"),
        total_tokens: event::usage_token(response, "total_tokens"),
        response_time: end.signed_duration_since(*start).num_milliseconds(),
        prompt: kwargs.get("messages"),
        tools,
        functions,
        additional: additional_fields(kwargs)?,
    })
}

#[derive(Debug, thiserror::Error)]
enum AthinaFailure<E>
where
    E: StdError + 'static,
{
    #[error(transparent)]
    Payload(#[from] PayloadError),
    #[error(transparent)]
    Delivery(#[from] DeliveryError<E>),
}

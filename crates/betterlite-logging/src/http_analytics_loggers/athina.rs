use std::error::Error as StdError;
use std::str;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
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
        let value = HeaderValue::try_from(api_key.expose_secret()).map_err(|source| {
            DeliveryError::Header {
                name: "athina-api-key",
                source,
            }
        })?;
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

fn payload(
    kwargs: &IndexMap<String, Json>,
    response: &Json,
    start: &DateTime<Utc>,
    end: &DateTime<Utc>,
) -> Result<Json, PayloadError> {
    let mut data = IndexMap::new();
    data.insert(
        "language_model_id".to_owned(),
        kwargs.get("model").cloned().unwrap_or(Json::Null),
    );
    data.insert("request".to_owned(), Json::Object(kwargs.clone()));
    data.insert("response".to_owned(), response.clone());
    insert_usage(&mut data, response);
    data.insert(
        "response_time".to_owned(),
        Json::Number(Number::from(
            end.signed_duration_since(*start).num_milliseconds(),
        )),
    );
    if let Some(messages) = kwargs.get("messages") {
        data.insert("prompt".to_owned(), messages.clone());
    }
    insert_optional_params(&mut data, kwargs);
    insert_additional_metadata(&mut data, kwargs)?;
    Ok(Json::Object(data))
}

fn insert_usage(data: &mut IndexMap<String, Json>, response: &Json) {
    for name in ["prompt_tokens", "completion_tokens", "total_tokens"] {
        data.insert(name.to_owned(), event::usage_token(response, name));
    }
}

fn insert_optional_params(data: &mut IndexMap<String, Json>, kwargs: &IndexMap<String, Json>) {
    if let Some(Json::Object(optional)) = kwargs.get("optional_params") {
        for name in ["tools", "functions"] {
            if let Some(value) = optional.get(name) {
                data.insert(name.to_owned(), value.clone());
            }
        }
    }
}

fn insert_additional_metadata(
    data: &mut IndexMap<String, Json>,
    kwargs: &IndexMap<String, Json>,
) -> Result<(), PayloadError> {
    let Some(metadata) = event::metadata(kwargs)? else {
        return Ok(());
    };
    if metadata.is_empty() {
        return Ok(());
    }
    for name in ADDITIONAL_KEYS {
        if let Some(value) = metadata.get(name) {
            data.insert(name.to_owned(), value.clone());
        }
    }
    Ok(())
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

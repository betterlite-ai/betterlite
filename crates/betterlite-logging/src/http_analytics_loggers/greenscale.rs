use std::error::Error as StdError;
use std::str;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use url::Url;

use super::clock::WallClock;
use super::delivery::{DELIVERY_DEADLINE, Delivery, DeliveryError, PostRequest};
use super::event::{self, Event, PayloadError};
use crate::python_json;

mod tests;

const TIMESTAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

pub struct GreenscaleLogger<D, C> {
    pub delivery: D,
    pub clock: C,
    pub settings: GreenscaleSettings,
}

pub struct GreenscaleSettings {
    pub api_key: Option<SecretString>,
    pub endpoint: Option<String>,
}

impl GreenscaleSettings {
    pub fn from_environment<E>(environment: &E) -> Self
    where
        E: EnvironmentLookup,
    {
        Self {
            api_key: environment
                .lookup("GREENSCALE_API_KEY")
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
            endpoint: environment
                .lookup("GREENSCALE_ENDPOINT")
                .filter(|value| !value.is_empty()),
        }
    }
}

impl<D: Delivery, C: WallClock> GreenscaleLogger<D, C> {
    pub async fn log_event(&self, event: &Event<'_>) {
        if let Err(error) = self.deliver(event).await {
            tracing::debug!(?error, "Greenscale logger error");
        }
    }

    async fn deliver(&self, event: &Event<'_>) -> Result<(), GreenscaleFailure<D::Error>> {
        let kwargs = event::kwargs_object(event.kwargs)?;
        let response = event::response_dump(event.response);
        let payload = self.payload(kwargs, &response, event)?;
        let body = serde_json::to_vec(&payload).map_err(DeliveryError::Encode)?;
        let sent = self
            .delivery
            .post(self.request(body)?)
            .await
            .map_err(DeliveryError::Transport)?;
        // LiteLLM logs the response text for any status and only a 200 counts as success
        let text = str::from_utf8(&sent.body).unwrap_or_default();
        if sent.status == 200 {
            tracing::debug!(status = sent.status, text, "Greenscale logger succeeded");
        } else {
            tracing::debug!(status = sent.status, text, "Greenscale logger error");
        }
        Ok(())
    }

    fn request(&self, body: Vec<u8>) -> Result<PostRequest, DeliveryError<D::Error>> {
        let Some(endpoint) = &self.settings.endpoint else {
            return Err(DeliveryError::MissingSetting {
                name: "GREENSCALE_ENDPOINT",
            });
        };
        let Some(api_key) = &self.settings.api_key else {
            return Err(DeliveryError::MissingSetting {
                name: "GREENSCALE_API_KEY",
            });
        };
        let url = Url::parse(endpoint).map_err(|source| DeliveryError::Url {
            name: "GREENSCALE_ENDPOINT",
            source,
        })?;
        let mut value = HeaderValue::try_from(api_key.expose_secret()).map_err(|source| {
            DeliveryError::Header {
                name: "api-key",
                source,
            }
        })?;
        value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(HeaderName::from_static("api-key"), value);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(PostRequest {
            url,
            headers,
            body,
            deadline: DELIVERY_DEADLINE,
        })
    }

    fn payload<'call>(
        &self,
        kwargs: &'call IndexMap<String, Json>,
        response: &Json,
        event: &Event<'_>,
    ) -> Result<GreenscalePayload<'call>, PayloadError> {
        let (project, application, tags) =
            event::metadata(kwargs)?.map_or_else(|| (None, None, Vec::new()), metadata_fields);
        Ok(GreenscalePayload {
            model_id: kwargs.get("model").cloned().unwrap_or(Json::Null),
            input_token_count: event::usage_token(response, "prompt_tokens"),
            output_token_count: event::usage_token(response, "completion_tokens"),
            timestamp: self.clock.now_utc().format(TIMESTAMP_FORMAT).to_string(),
            invocation_latency: event
                .end_time
                .signed_duration_since(event.start_time)
                .num_milliseconds(),
            project,
            application,
            tags,
        })
    }
}

#[derive(Serialize)]
struct GreenscalePayload<'call> {
    #[serde(rename = "modelId")]
    model_id: Json,
    #[serde(rename = "inputTokenCount")]
    input_token_count: Json,
    #[serde(rename = "outputTokenCount")]
    output_token_count: Json,
    timestamp: String,
    #[serde(rename = "invocationLatency")]
    invocation_latency: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<&'call Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    application: Option<&'call Json>,
    tags: Vec<Tag>,
}

#[derive(Serialize)]
struct Tag {
    key: String,
    value: String,
}

fn metadata_fields(metadata: &IndexMap<String, Json>) -> (Option<&Json>, Option<&Json>, Vec<Tag>) {
    let mut project = None;
    let mut application = None;
    let mut tags = Vec::new();
    for (name, value) in metadata {
        if !name.starts_with("greenscale") {
            continue;
        }
        match name.as_str() {
            "greenscale_project" => project = Some(value),
            "greenscale_application" => application = Some(value),
            _ => tags.push(Tag {
                key: name.replace("greenscale_", ""),
                value: tag_value(value),
            }),
        }
    }
    (project, application, tags)
}

// Python's str keeps numbers unchanged and writes everything else as JSON
fn tag_value(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        other
        @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            python_json::dumps(other)
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum GreenscaleFailure<E>
where
    E: StdError + 'static,
{
    #[error(transparent)]
    Payload(#[from] PayloadError),
    #[error(transparent)]
    Delivery(#[from] DeliveryError<E>),
}

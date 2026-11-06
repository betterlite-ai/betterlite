use std::error::Error as StdError;
use std::str;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use url::Url;

use super::clock::WallClock;
use super::delivery::{DELIVERY_DEADLINE, Delivery, DeliveryError, PostRequest};
use super::event::{self, Event, PayloadError};

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
        let value = HeaderValue::try_from(api_key.expose_secret()).map_err(|source| {
            DeliveryError::Header {
                name: "api-key",
                source,
            }
        })?;
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

    fn payload(
        &self,
        kwargs: &IndexMap<String, Json>,
        response: &Json,
        event: &Event<'_>,
    ) -> Result<Json, PayloadError> {
        let mut data = IndexMap::new();
        data.insert(
            "modelId".to_owned(),
            kwargs.get("model").cloned().unwrap_or(Json::Null),
        );
        data.insert(
            "inputTokenCount".to_owned(),
            event::usage_token(response, "prompt_tokens"),
        );
        data.insert(
            "outputTokenCount".to_owned(),
            event::usage_token(response, "completion_tokens"),
        );
        data.insert(
            "timestamp".to_owned(),
            Json::Text(self.clock.now_utc().format(TIMESTAMP_FORMAT).to_string()),
        );
        data.insert(
            "invocationLatency".to_owned(),
            Json::Number(Number::from(
                event
                    .end_time
                    .signed_duration_since(event.start_time)
                    .num_milliseconds(),
            )),
        );
        let tags = event::metadata(kwargs)?
            .map_or_else(Vec::new, |metadata| metadata_fields(metadata, &mut data));
        data.insert("tags".to_owned(), Json::List(tags));
        Ok(Json::Object(data))
    }
}

fn metadata_fields(
    metadata: &IndexMap<String, Json>,
    data: &mut IndexMap<String, Json>,
) -> Vec<Json> {
    let mut tags = Vec::new();
    for (name, value) in metadata {
        if !name.starts_with("greenscale") {
            continue;
        }
        match name.as_str() {
            "greenscale_project" => {
                data.insert("project".to_owned(), value.clone());
            }
            "greenscale_application" => {
                data.insert("application".to_owned(), value.clone());
            }
            _ => tags.push(tag(name, value)),
        }
    }
    tags
}

fn tag(name: &str, value: &Json) -> Json {
    let mut tag = IndexMap::new();
    tag.insert(
        "key".to_owned(),
        Json::Text(name.replace("greenscale_", "")),
    );
    tag.insert("value".to_owned(), Json::Text(event::tag_text(value)));
    Json::Object(tag)
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

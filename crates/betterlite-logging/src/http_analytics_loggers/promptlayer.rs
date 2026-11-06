use std::error::Error as StdError;
use std::str;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use url::Url;

use super::delivery::{DELIVERY_DEADLINE, Delivery, DeliveryError, PostRequest, PostResponse};
use super::event::{self, Event, PayloadError};

mod tests;

const TRACK_REQUEST_URL: &str = "https://api.promptlayer.com/rest/track-request";
const TRACK_METADATA_URL: &str = "https://api.promptlayer.com/rest/track-metadata";
const FUNCTION_NAME: &str = "openai.ChatCompletion.create";

pub struct PromptLayerLogger<D> {
    pub delivery: D,
    pub settings: PromptLayerSettings,
}

pub struct PromptLayerSettings {
    pub api_key: Option<SecretString>,
}

impl PromptLayerSettings {
    pub fn from_environment<E>(environment: &E) -> Self
    where
        E: EnvironmentLookup,
    {
        Self {
            api_key: environment
                .lookup("PROMPTLAYER_API_KEY")
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
        }
    }
}

impl<D: Delivery> PromptLayerLogger<D> {
    pub async fn log_event(&self, event: &Event<'_>) {
        if let Err(error) = self.deliver(event).await {
            tracing::debug!(?error, "PromptLayer logger error");
        }
    }

    async fn deliver(&self, event: &Event<'_>) -> Result<(), PromptLayerFailure<D::Error>> {
        let kwargs = event::kwargs_object(event.kwargs)?;
        let tracked = self.track_request(kwargs, event)?;
        let sent = self.post(TRACK_REQUEST_URL, tracked.body).await?;
        let response = Self::confirm(&sent)?;
        if let Some(request_id) = response.get("request_id")
            && !tracked.metadata.is_empty()
        {
            let mut trace = IndexMap::new();
            trace.insert("request_id".to_owned(), request_id.clone());
            trace.insert("api_key".to_owned(), self.api_key());
            trace.insert("metadata".to_owned(), Json::Object(tracked.metadata));
            self.post(TRACK_METADATA_URL, trace).await?;
        }
        Ok(())
    }

    fn track_request(
        &self,
        kwargs: &IndexMap<String, Json>,
        event: &Event<'_>,
    ) -> Result<TrackRequest, PromptLayerFailure<D::Error>> {
        let fields = call_fields(kwargs)?;
        let Some(Json::Object(response)) = event.response else {
            return Err(PromptLayerFailure::Payload(PayloadError::Response));
        };
        let mut new_kwargs = IndexMap::new();
        new_kwargs.insert("model".to_owned(), fields.model.clone());
        new_kwargs.insert("messages".to_owned(), fields.messages.clone());
        new_kwargs.extend(
            fields
                .optional
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        let (tags, metadata) = split_metadata(fields.stored);
        let mut body = IndexMap::new();
        body.insert(
            "function_name".to_owned(),
            Json::Text(FUNCTION_NAME.to_owned()),
        );
        body.insert("kwargs".to_owned(), Json::Object(new_kwargs));
        body.insert("tags".to_owned(), tags);
        body.insert(
            "request_response".to_owned(),
            Json::Object(response.clone()),
        );
        body.insert(
            "request_start_time".to_owned(),
            Json::Number(Number::from(event.start_time.timestamp())),
        );
        body.insert(
            "request_end_time".to_owned(),
            Json::Number(Number::from(event.end_time.timestamp())),
        );
        body.insert("api_key".to_owned(), self.api_key());
        Ok(TrackRequest { body, metadata })
    }

    async fn post(
        &self,
        target: &'static str,
        body: IndexMap<String, Json>,
    ) -> Result<PostResponse, PromptLayerFailure<D::Error>> {
        let encoded = serde_json::to_vec(&Json::Object(body)).map_err(DeliveryError::Encode)?;
        let request = Self::request(target, encoded)?;
        let sent = self
            .delivery
            .post(request)
            .await
            .map_err(DeliveryError::Transport)?;
        if !(200..300).contains(&sent.status) {
            return Err(PromptLayerFailure::Delivery(DeliveryError::Status {
                status: sent.status,
            }));
        }
        let text = str::from_utf8(&sent.body).unwrap_or_default();
        tracing::debug!(status = sent.status, text, "PromptLayer logger succeeded");
        Ok(sent)
    }

    fn request(
        target: &'static str,
        body: Vec<u8>,
    ) -> Result<PostRequest, DeliveryError<D::Error>> {
        let url = Url::parse(target).map_err(|source| DeliveryError::Url {
            name: "PromptLayer",
            source,
        })?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(PostRequest {
            url,
            headers,
            body,
            deadline: DELIVERY_DEADLINE,
        })
    }

    fn api_key(&self) -> Json {
        self.settings
            .api_key
            .as_ref()
            .map_or(Json::Null, |key| Json::Text(key.expose_secret().to_owned()))
    }

    fn confirm(
        sent: &PostResponse,
    ) -> Result<IndexMap<String, Json>, PromptLayerFailure<D::Error>> {
        let parsed: Json =
            serde_json::from_slice(&sent.body).map_err(|source| DeliveryError::Response {
                name: "PromptLayer",
                source,
            })?;
        let Json::Object(fields) = parsed else {
            return Err(PromptLayerFailure::Response);
        };
        if !matches!(fields.get("success"), Some(Json::Bool(true))) {
            return Err(PromptLayerFailure::Unsuccessful);
        }
        Ok(fields)
    }
}

struct TrackRequest {
    body: IndexMap<String, Json>,
    metadata: IndexMap<String, Json>,
}

struct CallFields<'call> {
    model: &'call Json,
    messages: &'call Json,
    optional: &'call IndexMap<String, Json>,
    stored: Option<&'call IndexMap<String, Json>>,
}

fn call_fields(kwargs: &IndexMap<String, Json>) -> Result<CallFields<'_>, PayloadError> {
    let model = kwargs
        .get("model")
        .ok_or(PayloadError::Missing { field: "model" })?;
    let messages = kwargs
        .get("messages")
        .ok_or(PayloadError::Missing { field: "messages" })?;
    let optional =
        event::object_field(kwargs, "optional_params")?.ok_or(PayloadError::Missing {
            field: "optional_params",
        })?;
    let params = event::object_field(kwargs, "litellm_params")?.ok_or(PayloadError::Missing {
        field: "litellm_params",
    })?;
    Ok(CallFields {
        model,
        messages,
        optional,
        stored: event::object_field(params, "metadata")?,
    })
}

fn split_metadata(metadata: Option<&IndexMap<String, Json>>) -> (Json, IndexMap<String, Json>) {
    let Some(stored) = metadata else {
        return (Json::List(Vec::new()), IndexMap::new());
    };
    let tags = stored
        .get("pl_tags")
        .cloned()
        .unwrap_or(Json::List(Vec::new()));
    let remaining = stored
        .iter()
        .filter(|(name, _value)| name.as_str() != "pl_tags")
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    (tags, remaining)
}

#[derive(Debug, thiserror::Error)]
enum PromptLayerFailure<E>
where
    E: StdError + 'static,
{
    #[error(transparent)]
    Payload(#[from] PayloadError),
    #[error(transparent)]
    Delivery(#[from] DeliveryError<E>),
    #[error("PromptLayer did not confirm the request")]
    Unsuccessful,
    #[error("the PromptLayer response is not a JSON object")]
    Response,
}

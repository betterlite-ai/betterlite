use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::ExposeSecret as _;
use url::Url;

use crate::gcs_mock::auth::{EnsureAccessToken, GetTokenAndUrl, VertexAuth, VertexAuthProvider};
use crate::gcs_pubsub::{GcsPubSubLogger, PubSubError};
use crate::http_analytics_loggers::delivery::{DELIVERY_DEADLINE, Delivery, PostRequest};

const TOKEN_MODEL: &str = "pub-sub";
const JSON_CONTENT_TYPE: &str = "application/json";
const ACCEPTED_STATUSES: [u16; 2] = [200, 202];

pub(crate) async fn publish<Auth, D>(
    logger: &GcsPubSubLogger<Auth, D>,
    message: &Json,
) -> Result<(), PubSubError<Auth, D>>
where
    Auth: VertexAuth,
    D: Delivery,
{
    let headers = construct_request_headers(logger).await?;
    let data = message_text(message).map_err(|source| PubSubError::Json {
        record: "message",
        source,
    })?;
    let body = request_body(&data).map_err(|source| PubSubError::Json {
        record: "request body",
        source,
    })?;
    let request = PostRequest {
        url: publish_url(logger)?,
        headers,
        body,
        deadline: DELIVERY_DEADLINE,
    };
    let response = logger
        .delivery
        .post(request)
        .await
        .map_err(|source| PubSubError::Transport { source })?;
    if !ACCEPTED_STATUSES.contains(&response.status) {
        return Err(PubSubError::Status {
            status: response.status,
        });
    }
    tracing::debug!(status = response.status, "Pub/Sub response");
    Ok(())
}

async fn construct_request_headers<Auth, D>(
    logger: &GcsPubSubLogger<Auth, D>,
) -> Result<HeaderMap, PubSubError<Auth, D>>
where
    Auth: VertexAuth,
    D: Delivery,
{
    let access = logger
        .auth
        .ensure_access_token_async(EnsureAccessToken {
            credentials: logger.settings.credentials.clone(),
            project_id: Some(logger.settings.project_id.clone()),
            custom_llm_provider: VertexAuthProvider::VertexAi,
        })
        .await
        .map_err(|source| PubSubError::Headers { source })?;
    let token = logger
        .auth
        .get_token_and_url(GetTokenAndUrl {
            model: TOKEN_MODEL.to_owned(),
            auth_header: Some(access.token),
            vertex_credentials: logger.settings.credentials.clone(),
            vertex_project: Some(access.project_id),
            vertex_location: None,
            gemini_api_key: None,
            stream: None,
            custom_llm_provider: VertexAuthProvider::VertexAi,
            api_base: None,
        })
        .map_err(|source| PubSubError::Headers { source })?;
    let mut headers = HeaderMap::new();
    let bearer = token.token.expose_secret();
    let mut authorization = HeaderValue::from_str(&format!("Bearer {bearer}"))
        .map_err(|source| PubSubError::Header { source })?;
    authorization.set_sensitive(true);
    headers.insert(AUTHORIZATION, authorization);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE));
    Ok(headers)
}

fn publish_url<Auth, D>(logger: &GcsPubSubLogger<Auth, D>) -> Result<Url, PubSubError<Auth, D>>
where
    Auth: VertexAuth,
    D: Delivery,
{
    let project = &logger.settings.project_id;
    let topic = &logger.settings.topic_id;
    Url::parse(&format!(
        "https://pubsub.googleapis.com/v1/projects/{project}/topics/{topic}:publish"
    ))
    .map_err(|source| PubSubError::Url { source })
}

// json.dumps(message, default=str): a string stays as it is, and every other value is written as JSON
fn message_text(message: &Json) -> Result<String, serde_json::Error> {
    match message {
        Json::Text(text) => Ok(text.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(message)
        }
    }
}

// the Pub/Sub publish body: one message entry holding the base64 text of the encoded message
fn request_body(data: &str) -> Result<Vec<u8>, serde_json::Error> {
    let mut entry = JsonObject::new();
    entry.insert("data".to_owned(), Json::Text(STANDARD.encode(data)));
    let mut body = JsonObject::new();
    body.insert("messages".to_owned(), Json::List(vec![Json::Object(entry)]));
    serde_json::to_vec(&Json::Object(body))
}

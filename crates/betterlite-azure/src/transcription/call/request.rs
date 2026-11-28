use std::time::Duration;

use betterlite_http::client_kind::ClientKind;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::transcription::client::TranscriptionClient;
use betterlite_openai::transcription::request::{
    TranscriptionData, outbound_request as openai_request,
};
use http::Method;
use http::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::AzureTranscriptionError;
use super::client::RequestAuth;
use crate::transcription::encoding::quote_plus;

// The SDK prefixes /deployments/{model} unless the base already names a deployment, then appends the endpoint.
fn request_url(
    base: &str,
    model: Option<&Json>,
    api_version: Option<&str>,
) -> Result<Url, AzureTranscriptionError> {
    let mut parsed =
        HttpxUrl::parse(base).map_err(|source| AzureTranscriptionError::Url { source })?;
    let base_query = parsed.query.take();
    parsed.fragment = None;
    let deployment = match model {
        Some(Json::Text(name)) => Some(name.as_str()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(AzureTranscriptionError::Field {
                field: String::from("model"),
                problem: "is not a string",
            });
        }
        None | Some(Json::Null) => None,
    };
    let prefix = match deployment {
        Some(name) if !parsed.path.contains("/deployments") => format!("deployments/{name}/"),
        Some(_) | None => String::new(),
    };
    let mut composed = HttpxUrl::parse(&format!("{}{prefix}audio/transcriptions", parsed.href()))
        .map_err(|source| AzureTranscriptionError::Url { source })?;
    composed.query = merged_query(base_query.as_deref(), api_version);
    Url::parse(&composed.href()).map_err(|source| AzureTranscriptionError::BaseUrl { source })
}

// The SDK merges the base URL's query with api-version, which wins when both name it.
fn merged_query(base_query: Option<&str>, api_version: Option<&str>) -> Option<String> {
    if base_query.is_none() && api_version.is_none() {
        return None;
    }
    let mut pairs: Vec<String> = base_query
        .map(|query| {
            query
                .split('&')
                .filter(|pair| !pair.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if let Some(version) = api_version {
        let encoded = format!("api-version={}", quote_plus(version));
        match pairs
            .iter_mut()
            .find(|pair| pair.split('=').next() == Some("api-version"))
        {
            Some(pair) => pair.clone_from(&encoded),
            None => pairs.push(encoded),
        }
    }
    Some(pairs.join("&"))
}

// what the selected client contributes to one request: its base, version, auth and deadline
pub struct OutboundTarget<'target> {
    pub base_url: &'target str,
    pub api_version: Option<&'target str>,
    pub auth: &'target RequestAuth,
    pub deadline: Duration,
}

// the api-key or Authorization header the SDK sends, and the secret it comes from
fn auth_headers(auth: &RequestAuth) -> Result<(HeaderMap, &SecretString), AzureTranscriptionError> {
    let (name, text, secret) = match auth {
        RequestAuth::ApiKey(key) => (
            HeaderName::from_static("api-key"),
            key.expose_secret().to_owned(),
            key,
        ),
        RequestAuth::Bearer(token) => (
            AUTHORIZATION,
            format!("Bearer {}", token.expose_secret()),
            token,
        ),
    };
    let mut value =
        HeaderValue::try_from(text).map_err(|source| AzureTranscriptionError::Header { source })?;
    value.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(name, value);
    Ok((headers, secret))
}

pub fn outbound_request(
    target: &OutboundTarget<'_>,
    data: &TranscriptionData,
    file: &ProcessedAudioFile,
    boundary: String,
) -> Result<OutboundRequest, AzureTranscriptionError> {
    let url = request_url(target.base_url, data.model.as_ref(), target.api_version)?;
    let (headers, secret) = auth_headers(target.auth)?;
    // AzureOpenAI writes the OpenAI SDK's multipart body, so B515's request builds it and only its URL and auth are replaced
    let shared = openai_request(
        &TranscriptionClient {
            kind: ClientKind::Sync,
            api_key: secret.clone(),
            base_url: url.clone(),
            timeout: target.deadline,
            max_retries: 0,
            shared_session: None,
        },
        data,
        file,
        boundary,
    )?;
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: shared.body,
        deadline: target.deadline,
    })
}

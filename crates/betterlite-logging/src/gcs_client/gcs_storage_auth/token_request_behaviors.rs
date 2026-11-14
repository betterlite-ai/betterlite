use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::gcs_mock::auth::{AccessToken, GetTokenAndUrl, VertexAuth, VertexAuthProvider};
use crate::gcs_mock::client::GcsHttpClient;

use super::error::GcsClientError;
use super::{
    AUTHORIZATION_HEADER, CONTENT_TYPE_HEADER, JSON_CONTENT_TYPE, RequestHeaders, TOKEN_MODEL,
    VertexInstance,
};

pub(super) fn token_headers<Auth, Http>(
    instance: &VertexInstance<Auth>,
    access: AccessToken,
    bucket_name: &str,
    service_account_json: Option<&SecretString>,
) -> Result<RequestHeaders, GcsClientError<Auth, Http>>
where
    Auth: VertexAuth,
    Http: GcsHttpClient,
{
    let token = instance
        .get_token_and_url(GetTokenAndUrl {
            model: TOKEN_MODEL.to_owned(),
            auth_header: Some(access.token),
            vertex_credentials: service_account_json.cloned(),
            vertex_project: Some(access.project_id),
            vertex_location: None,
            gemini_api_key: None,
            stream: None,
            custom_llm_provider: VertexAuthProvider::VertexAi,
            api_base: None,
        })
        .map_err(|source| header_error(bucket_name, source))?;
    tracing::debug!(bucket = %bucket_name, "constructed the GCS request authorization header");
    Ok(bearer_headers(&token.token))
}

pub(super) fn header_error<Auth, Http>(
    bucket_name: &str,
    source: Auth::Error,
) -> GcsClientError<Auth, Http>
where
    Auth: VertexAuth,
    Http: GcsHttpClient,
{
    GcsClientError::Headers {
        bucket_name: bucket_name.to_owned(),
        source,
    }
}

pub(super) fn to_header_map<Auth, Http>(
    headers: &RequestHeaders,
) -> Result<HeaderMap, GcsClientError<Auth, Http>>
where
    Auth: VertexAuth,
    Http: GcsHttpClient,
{
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.insert(
            HeaderName::from_bytes(name.as_bytes()).map_err(GcsClientError::HeaderName)?,
            HeaderValue::from_str(value.expose_secret()).map_err(GcsClientError::Header)?,
        );
    }
    Ok(map)
}

fn bearer_headers(token: &SecretString) -> RequestHeaders {
    let mut headers = RequestHeaders::new();
    headers.insert(
        AUTHORIZATION_HEADER.to_owned(),
        SecretString::from(format!("Bearer {}", token.expose_secret())),
    );
    headers.insert(
        CONTENT_TYPE_HEADER.to_owned(),
        SecretString::from(JSON_CONTENT_TYPE),
    );
    headers
}

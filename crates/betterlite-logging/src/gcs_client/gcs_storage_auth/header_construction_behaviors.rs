use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::gcs_mock::auth::{EnsureAccessToken, GetTokenAndUrl, VertexAuth, VertexAuthProvider};
use crate::gcs_mock::client::GcsHttpClient;

use super::error::GcsClientError;
use super::{
    AUTHORIZATION_HEADER, CONTENT_TYPE_HEADER, GcsBucketBase, JSON_CONTENT_TYPE, PROJECT_ID_ENV,
    RequestHeaders, TOKEN_MODEL, VertexInstance,
};

pub async fn construct_request_headers<Auth, Http>(
    base: &GcsBucketBase<Auth, Http>,
    bucket_name: &str,
    service_account_json: Option<&SecretString>,
    vertex_instance: Option<&VertexInstance<Auth>>,
) -> Result<RequestHeaders, GcsClientError<Auth, Http>>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let instance = vertex_instance.unwrap_or(&base.auth);
    let access = instance
        .ensure_access_token_async(EnsureAccessToken {
            credentials: service_account_json.cloned(),
            project_id: None,
            custom_llm_provider: VertexAuthProvider::VertexAi,
        })
        .await
        .map_err(|source| GcsClientError::Headers {
            bucket_name: bucket_name.to_owned(),
            source,
        })?;
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
        .map_err(|source| GcsClientError::Headers {
            bucket_name: bucket_name.to_owned(),
            source,
        })?;
    tracing::debug!(bucket = %bucket_name, "constructed the GCS request authorization header");
    Ok(bearer_headers(&token.token))
}

pub fn sync_construct_request_headers<Auth, Http>(
    base: &GcsBucketBase<Auth, Http>,
    bucket_name: &str,
    service_account_path: Option<&SecretString>,
) -> Result<RequestHeaders, GcsClientError<Auth, Http>>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let project_id = base.environment.get(PROJECT_ID_ENV);
    let access = base
        .auth
        .ensure_access_token(EnsureAccessToken {
            credentials: service_account_path.cloned(),
            project_id,
            custom_llm_provider: VertexAuthProvider::VertexAi,
        })
        .map_err(|source| GcsClientError::Headers {
            bucket_name: bucket_name.to_owned(),
            source,
        })?;
    let token = base
        .auth
        .get_token_and_url(GetTokenAndUrl {
            model: TOKEN_MODEL.to_owned(),
            auth_header: Some(access.token),
            vertex_credentials: service_account_path.cloned(),
            vertex_project: Some(access.project_id),
            vertex_location: None,
            gemini_api_key: None,
            stream: None,
            custom_llm_provider: VertexAuthProvider::VertexAi,
            api_base: None,
        })
        .map_err(|source| GcsClientError::Headers {
            bucket_name: bucket_name.to_owned(),
            source,
        })?;
    tracing::debug!(bucket = %bucket_name, "constructed the GCS request authorization header");
    Ok(bearer_headers(&token.token))
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

mod tests;

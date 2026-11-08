use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_context::local_ttl_cache::credential::{Credential, CredentialHandle};
use betterlite_context::local_ttl_cache::{CacheError, CacheValue};
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::Url;
use url::form_urlencoded::Serializer;

use super::WatsonxAuth;
use super::environment::{WatsonxEnvironment, first_set};
use super::error::WatsonxError;

pub const DEFAULT_IAM_URL: &str = "https://iam.cloud.ibm.com/identity/token";
const GRANT_TYPE: &str = "urn:ibm:params:oauth:grant-type:apikey";
// expires_in minus the ten second buffer, written as the negative addend so no float operator is needed
pub(super) const EXPIRY_BUFFER_OFFSET: f64 = -10.0;
// LiteLLM's default httpx read timeout for its cached clients
pub(super) const TOKEN_REQUEST_DEADLINE: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub struct IamTokenResponse {
    pub access_token: SecretString,
    pub expires_in: f64,
}

#[derive(Deserialize)]
struct RawIamTokenReply {
    #[serde(default)]
    access_token: Option<SecretString>,
    #[serde(default)]
    expires_in: Option<f64>,
}

#[must_use]
pub fn get_watsonx_iam_url(environment: &WatsonxEnvironment) -> String {
    environment
        .watsonx_iam_url
        .as_deref()
        .filter(|url| !url.is_empty())
        .map_or_else(|| DEFAULT_IAM_URL.to_owned(), str::to_owned)
}

pub fn decode_iam_token_response(bytes: &[u8]) -> Result<IamTokenResponse, WatsonxError> {
    let raw: RawIamTokenReply = serde_json::from_slice(bytes).map_err(WatsonxError::IamReply)?;
    let Some(access_token) = raw.access_token else {
        return Err(WatsonxError::IamReplyField {
            field: "access_token",
        });
    };
    let Some(expires_in) = raw.expires_in else {
        return Err(WatsonxError::IamReplyField {
            field: "expires_in",
        });
    };
    Ok(IamTokenResponse {
        access_token,
        expires_in,
    })
}

pub(super) fn iam_request(
    endpoint: &str,
    api_key: &SecretString,
) -> Result<OutboundRequest, WatsonxError> {
    let url = Url::parse(endpoint).map_err(WatsonxError::IamUrl)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    let form = Serializer::new(String::new())
        .append_pair("grant_type", GRANT_TYPE)
        .append_pair("apikey", api_key.expose_secret())
        .finish();
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Bytes {
            content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
            data: form.into_bytes(),
        },
        deadline: TOKEN_REQUEST_DEADLINE,
    })
}

pub(super) fn resolve_api_key(
    api_key: Option<&SecretString>,
    environment: &WatsonxEnvironment,
) -> Option<SecretString> {
    api_key.map_or_else(
        || {
            first_set(&[
                environment
                    .wx_api_key
                    .as_ref()
                    .map(ExposeSecret::expose_secret),
                environment
                    .watsonx_api_key
                    .as_ref()
                    .map(ExposeSecret::expose_secret),
                environment
                    .watsonx_apikey
                    .as_ref()
                    .map(ExposeSecret::expose_secret),
                environment
                    .watsonx_zenapikey
                    .as_ref()
                    .map(ExposeSecret::expose_secret),
            ])
            .map(|text| SecretString::from(text.to_owned()))
        },
        |key| Some(key.clone()),
    )
}

pub(super) fn cached_token<T>(
    auth: &WatsonxAuth<T>,
    key: &str,
) -> Result<Option<SecretString>, WatsonxError> {
    let cached = auth
        .cache
        .lock()
        .map_err(|_poisoned| WatsonxError::Cache(CacheError::Unavailable))?
        .get(key)
        .map_err(WatsonxError::Cache)?;
    Ok(match cached {
        Some(CacheValue::Credential(handle)) => Some(handle.0.token.clone()),
        Some(
            CacheValue::Bytes(_)
            | CacheValue::Null
            | CacheValue::Boolean(_)
            | CacheValue::Integer(_)
            | CacheValue::Float(_)
            | CacheValue::Text(_)
            | CacheValue::Array(_)
            | CacheValue::Object(_),
        )
        | None => None,
    })
}

pub(super) fn store_token<T>(
    auth: &WatsonxAuth<T>,
    key: &str,
    token: &SecretString,
    ttl: f64,
) -> Result<(), WatsonxError> {
    let credential = Credential {
        token: token.clone(),
        headers: BTreeMap::new(),
    };
    let handle = CredentialHandle(Arc::new(credential));
    let mut cache = auth
        .cache
        .lock()
        .map_err(|_poisoned| WatsonxError::Cache(CacheError::Unavailable))?;
    cache
        .set(key, CacheValue::Credential(handle), Some(ttl))
        .map_err(WatsonxError::Cache)
}

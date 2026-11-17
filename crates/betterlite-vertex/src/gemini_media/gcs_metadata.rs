use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::credential_source::{
    ConfiguredCredentials, CredentialDecodeError,
};
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_http::url_path::quote_segment;
use betterlite_prompts::arguments::json::{Json, Object};
use http::Method;
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue, InvalidHeaderValue};
use secrecy::ExposeSecret as _;
use thiserror::Error;
use url::Url;

use crate::token_cache::error::CredentialCacheError;
use crate::vertex_interactions::mint::MintAccessToken;

const GS_SCHEME: &str = "gs://";
const CONTENT_TYPE_FIELD: &str = "contentType";
const METADATA_PREFIX: &str = "https://storage.googleapis.com/storage/v1/b/";
const MIN_BUCKET_LENGTH: usize = 3;
const MAX_BUCKET_LENGTH: usize = 63;
const MAX_DOTTED_BUCKET_LENGTH: usize = 222;
const MAX_LABEL_LENGTH: usize = 63;
const PROJECT_ALIASES: [&str; 2] = ["vertex_project", "vertex_ai_project"];
const CREDENTIALS_ALIASES: [&str; 2] = ["vertex_credentials", "vertex_ai_credentials"];

// LiteLLM's shared metadata HTTPHandler timeout, which bounds the whole lookup here
pub const METADATA_DEADLINE: Duration = Duration::from_secs(5);

pub type MetadataFuture<'fetch> =
    Pin<Box<dyn Future<Output = Result<UpstreamResponse, TransportError>> + Send + 'fetch>>;

pub trait GcsTransport: Send + Sync {
    fn get(&self, request: OutboundRequest) -> MetadataFuture<'_>;
}

impl GcsTransport for ProxiedClient {
    fn get(&self, request: OutboundRequest) -> MetadataFuture<'_> {
        Box::pin(async move {
            // LiteLLM's httpx handlers follow redirects, so every hop is re-checked by the shared client
            self.send_following(request)
                .await?
                .buffered(self.limits.body_bytes)
                .await
        })
    }
}

pub struct GcsDeps<'deps> {
    pub transport: &'deps dyn GcsTransport,
    pub tokens: &'deps dyn MintAccessToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthValues<'params> {
    pub project: Option<&'params Json>,
    pub credentials: Option<&'params Json>,
}

#[derive(Debug, Error)]
pub enum GcsMetadataError {
    #[error("the vertex project is not a string")]
    Project,
    #[error("the vertex credentials must be a string or an object")]
    Credentials,
    #[error("the vertex credentials could not be decoded")]
    DecodeCredentials(#[source] CredentialDecodeError),
    #[error("the inline vertex credentials could not be written")]
    WriteCredentials(#[source] serde_json::Error),
    #[error("the metadata access token could not be minted")]
    Mint(#[source] CredentialCacheError),
    #[error("the metadata access token is not a header value")]
    Token(#[source] InvalidHeaderValue),
    #[error("the metadata request could not be sent")]
    Transport(#[source] TransportError),
    #[error("the metadata request answered HTTP {0}")]
    Status(u16),
    #[error("the metadata reply is not JSON")]
    Decode(#[source] serde_json::Error),
    #[error("the metadata reply is not an object")]
    NotAnObject,
    #[error("the metadata reply has no usable contentType")]
    MissingContentType,
}

#[must_use]
pub fn auth_values(params: &Object) -> AuthValues<'_> {
    AuthValues {
        project: alias_or(params, &PROJECT_ALIASES),
        credentials: alias_or(params, &CREDENTIALS_ALIASES),
    }
}

// Python's `a or b` over the aliases: a falsy value falls through, and a chain with no truthy value is absent
fn alias_or<'params>(params: &'params Object, aliases: &[&str]) -> Option<&'params Json> {
    aliases
        .iter()
        .find_map(|name| params.get(*name).filter(|value| value.is_truthy()))
}

// LiteLLM's GCS core naming constraints; the bucket is validated before any auth is minted or sent
#[must_use]
pub fn bucket_is_valid(bucket: &str) -> bool {
    let dotted = bucket.contains('.');
    let longest = if dotted {
        MAX_DOTTED_BUCKET_LENGTH
    } else {
        MAX_BUCKET_LENGTH
    };
    let size = bucket.chars().count();
    if size < MIN_BUCKET_LENGTH || size > longest {
        return false;
    }
    // the label rule is vacuous for a dotless name, which the length check above already bounds to 63 characters
    if bucket
        .split('.')
        .any(|part| part.is_empty() || part.chars().count() > MAX_LABEL_LENGTH)
    {
        return false;
    }
    let boundary = |character: char| character.is_ascii_lowercase() || character.is_ascii_digit();
    if !bucket.chars().next().is_some_and(boundary)
        || !bucket.chars().next_back().is_some_and(boundary)
    {
        return false;
    }
    if !bucket
        .chars()
        .all(|character| boundary(character) || matches!(character, '.' | '_' | '-'))
    {
        return false;
    }
    let quads = bucket.split('.').count() == 4
        && bucket
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    !bucket.contains("..") && !quads
}

#[must_use]
pub fn metadata_url(gs_uri: &str) -> Option<Url> {
    let rest = gs_uri.strip_prefix(GS_SCHEME)?;
    let (bucket, object) = rest.split_once('/')?;
    if bucket.is_empty() || object.is_empty() || !bucket_is_valid(bucket) {
        return None;
    }
    let encoded = format!(
        "{METADATA_PREFIX}{}/o/{}?fields={CONTENT_TYPE_FIELD}",
        quote_segment(bucket),
        quote_segment(object)
    );
    Url::parse(&encoded).ok()
}

// An explicit lookup reports its failures; an anonymous one answers None, so only public objects resolve
pub async fn object_content_type(
    gs_uri: &str,
    auth: AuthValues<'_>,
    deps: &GcsDeps<'_>,
) -> Result<Option<String>, GcsMetadataError> {
    let Some(url) = metadata_url(gs_uri) else {
        return Ok(None);
    };
    let explicit = auth.project.is_some() || auth.credentials.is_some();
    let headers = if explicit {
        bearer_headers(auth, deps).await?
    } else {
        HeaderMap::new()
    };
    let request = OutboundRequest {
        method: Method::GET,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Empty,
        deadline: METADATA_DEADLINE,
    };
    let reply = match deps.transport.get(request).await {
        Ok(reply) => reply,
        Err(source) => return failed(explicit, GcsMetadataError::Transport(source)),
    };
    if reply.status >= 400 {
        return failed(explicit, GcsMetadataError::Status(reply.status));
    }
    read_content_type(&reply.body, explicit)
}

async fn bearer_headers(
    auth: AuthValues<'_>,
    deps: &GcsDeps<'_>,
) -> Result<HeaderMap, GcsMetadataError> {
    let project = match auth.project {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(_) => return Err(GcsMetadataError::Project),
        None => None,
    };
    let credentials = match auth.credentials {
        None => None,
        Some(Json::Text(text)) => Some(
            ConfiguredCredentials::from_text(text.clone())
                .map_err(GcsMetadataError::DecodeCredentials)?,
        ),
        Some(value @ Json::Object(_)) => {
            let text = serde_json::to_string(value).map_err(GcsMetadataError::WriteCredentials)?;
            Some(
                ConfiguredCredentials::from_text(text)
                    .map_err(GcsMetadataError::DecodeCredentials)?,
            )
        }
        Some(_) => return Err(GcsMetadataError::Credentials),
    };
    let access = deps
        .tokens
        .mint(credentials.as_ref(), project)
        .await
        .map_err(GcsMetadataError::Mint)?;
    let token = access.token.expose_secret();
    let mut value =
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(GcsMetadataError::Token)?;
    value.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
}

pub fn read_content_type(body: &[u8], explicit: bool) -> Result<Option<String>, GcsMetadataError> {
    let value: Json = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(source) => return failed(explicit, GcsMetadataError::Decode(source)),
    };
    let Json::Object(fields) = value else {
        return failed(explicit, GcsMetadataError::NotAnObject);
    };
    if let Some(text) = fields.get(CONTENT_TYPE_FIELD).and_then(Json::as_str)
        && !text.is_empty()
    {
        return Ok(Some(text.to_owned()));
    }
    failed(explicit, GcsMetadataError::MissingContentType)
}

fn failed(explicit: bool, error: GcsMetadataError) -> Result<Option<String>, GcsMetadataError> {
    if explicit { Err(error) } else { Ok(None) }
}

// the request tests drive the same scripted transport and mint, so the doubles live here and both modules read them
pub(crate) mod tests;

use std::collections::BTreeMap;
use std::fmt;

use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_config::sections::request_body_safety::ClientsideParamGrant;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use thiserror::Error;

use crate::request_ban_catalog::RequestBanCatalog;

pub mod banned;
pub mod destination;
pub mod fallback;
pub mod grants;
pub mod metadata;

mod tests;

pub const MAX_METADATA_BYTES: usize = 0x0010_0000;
pub const MAX_METADATA_DEPTH: usize = 64;

// The metadata container a caller may send as a JSON string; the shared loader bounds the bytes and depth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedValue {
    pub json: Json,
}

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the metadata JSON is {size} bytes, over the {limit}-byte limit")]
    TooLarge { size: usize, limit: usize },
    #[error("the metadata JSON nests past {limit} levels")]
    TooDeep { limit: usize },
    #[error("the metadata is not valid JSON")]
    Malformed {
        #[source]
        source: serde_json::Error,
    },
}

// Both the object and the stringified object are byte- and depth-bounded before any scan reads them.
pub fn decode_request_metadata(bytes: &[u8]) -> Result<TypedValue, DecodeError> {
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(DecodeError::TooLarge {
            size: bytes.len(),
            limit: MAX_METADATA_BYTES,
        });
    }
    if byte_depth(bytes) > MAX_METADATA_DEPTH {
        return Err(DecodeError::TooDeep {
            limit: MAX_METADATA_DEPTH,
        });
    }
    let json = serde_json::from_slice::<Json>(bytes)
        .map_err(|source| DecodeError::Malformed { source })?;
    Ok(TypedValue { json })
}

// Counts container nesting on raw bytes before the parser recurses; brackets in strings are text.
fn byte_depth(bytes: &[u8]) -> usize {
    let mut depth = 0_usize;
    let mut deepest = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for &byte in bytes {
        if quoted {
            match byte {
                b'\\' if !escaped => escaped = true,
                b'"' if !escaped => quoted = false,
                _ => escaped = false,
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'[' | b'{' => {
                depth = depth.saturating_add(1);
                deepest = deepest.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

// None when no model group carries the name, Some(None) when the group has no opt-in of its own.
#[derive(Debug, Clone, Copy)]
pub enum ModelGrants<'grants> {
    Absent,
    Unset,
    Granted(&'grants [ClientsideParamGrant]),
}

pub trait ModelGrantLookup: fmt::Debug + Send + Sync {
    fn grants(&self, model_group: &str) -> ModelGrants<'_>;
}

// The extractor's top-level names only: a URL-valued field is refused by name before any opt-in.
pub trait FormMetadataExtractor: fmt::Debug + Send + Sync {
    fn top_level_names(&self, form_data: &JsonObject, prefix: &str) -> Vec<String>;
}

#[derive(Debug)]
pub struct RequestBodySafety<'scan> {
    pub body: &'scan JsonObject,
    pub model: &'scan str,
    pub allow_client_side_credentials: bool,
    // loaded for compatibility with provider_url_destination_allowed_hosts; betterlite never lets it authorize
    pub allowed_hosts: &'scan [AllowedDestination],
    pub grants: &'scan dyn ModelGrantLookup,
    pub metadata: &'scan dyn FormMetadataExtractor,
    pub max_fallbacks: i64,
}

pub fn is_request_body_safe(scan: &RequestBodySafety<'_>) -> Result<(), SafetyError> {
    // LiteLLM refuses the SDK-only field before every opt-in, so a proxy-wide opt-in cannot admit it either.
    if scan.body.contains_key("model_list") {
        return Err(SafetyError::ModelList);
    }
    let catalog = RequestBanCatalog::pinned();
    banned::check(scan, scan.body, &catalog)?;
    for key in catalog.nested_config {
        if let Some(container) = metadata::coerce(scan.body.get(key))? {
            banned::check(scan, &container, &catalog)?;
        }
    }
    for key in catalog.metadata {
        if let Some(container) = metadata::coerce(scan.body.get(key))? {
            banned::check(scan, &container, &catalog)?;
        }
        let prefix = format!("{key}[");
        if scan.body.keys().any(|name| name.starts_with(&prefix)) {
            let names = scan.metadata.top_level_names(scan.body, &prefix);
            // presence is the whole check here; a matcher cannot authorize api_base, so values are null
            let bracket: JsonObject = names.into_iter().map(|name| (name, Json::Null)).collect();
            banned::check(scan, &bracket, &catalog)?;
        }
    }
    fallback::check(scan, &catalog)?;
    if let Some(params) = metadata::coerce(scan.body.get("litellm_params"))? {
        if let Some(nested) = metadata::coerce(params.get("metadata"))? {
            banned::check(scan, &nested, &catalog)?;
        }
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum SafetyError {
    #[error("Rejected Request: model_list is not allowed in the request body.")]
    ModelList,
    #[error(
        "Rejected Request: {param} is not allowed in request body. Clientside passthrough requires explicit \
         admin opt-in via either `general_settings.allow_client_side_credentials = true` (proxy-wide) or \
         `configurable_clientside_auth_params` on the deployment in your proxy config.yaml. Relevant Issue: \
         https://huntr.com/bounties/4001e1a2-7b7a-4776-a3ae-e6692ec3d997"
    )]
    Banned { param: String },
    #[error("Rejected Request: fallback nesting exceeds the allowed validation depth.")]
    FallbackDepth,
    // the destination value stays out of the message: it may carry caller credentials
    #[error("Rejected Request: a URL-valued fallback destination is not allowed.")]
    FallbackDestination,
    #[error("Rejected Request: the api_base matcher has no api_base entry.")]
    MatcherMissing,
    #[error("Rejected Request: the api_base matcher and the request value must both be strings.")]
    MatcherType,
    #[error("Rejected Request: the api_base matcher is not a valid regex.")]
    MatcherRegex(#[source] pcre2::Error),
    #[error("Rejected Request: {error}")]
    MetadataLimit {
        #[source]
        error: Box<DecodeError>,
    },
}

impl SafetyError {
    // A bare exception during auth becomes auth_error with param "None" and status 401.
    #[must_use]
    pub fn into_proxy_exception(self) -> ProxyException {
        ProxyException::new(ProxyExceptionInit {
            message: format!("Authentication Error, {self}"),
            error_type: "auth_error".to_owned(),
            param: Some("None".to_owned()),
            code: ExceptionCode::Status(401),
            headers: BTreeMap::new(),
            openai_code: None,
            provider_specific_fields: BTreeMap::new(),
        })
    }
}

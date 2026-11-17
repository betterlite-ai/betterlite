use betterlite_openai::openai_base_config::Headers;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;

use crate::operation::OperationError;
use crate::prepared::body::{authorization_value, header_value};
use crate::prepared::deployment::Deployment;

use super::call::Composition;
use super::error::OpenAiProviderError;

// completion() takes the supplied headers or else the extra headers, then litellm.headers, and the extra headers win a shared name.
#[must_use]
pub fn resolved_headers(
    composition: &Composition,
    supplied: Option<&Headers>,
    extra: Option<&Headers>,
) -> Option<Headers> {
    let base = supplied
        .filter(|map| !map.is_empty())
        .or_else(|| extra.filter(|map| !map.is_empty()))
        .or_else(|| {
            composition
                .globals_headers
                .as_ref()
                .filter(|map| !map.is_empty())
        });
    merge_headers(base, extra)
}

// The first map is the base and the second wins a shared name; an absent map keeps the other one as it is.
#[must_use]
pub fn merge_headers(base: Option<&Headers>, over: Option<&Headers>) -> Option<Headers> {
    let mut merged = base.cloned();
    if let Some(overrides) = over {
        merged.get_or_insert_default().extend(overrides.clone());
    }
    merged
}

pub fn chat_headers(
    deployment: &Deployment,
    supplied: Option<&Headers>,
    experimental: bool,
) -> Result<HeaderMap, OpenAiProviderError> {
    let authorization = deployment
        .credential()
        .ok_or(OpenAiProviderError::Operation(
            OperationError::MissingApiKey,
        ))?;
    let authorization_header = authorization_value(authorization.expose_secret())?;
    let mut headers = HeaderMap::new();
    // the base handler takes the caller's headers, adds Content-Type when absent, and sets the key over them
    if experimental {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        insert_headers(&mut headers, supplied)?;
        headers.insert(AUTHORIZATION, authorization_header);
        return Ok(headers);
    }
    // the OpenAI SDK sends JSON for a stream too, and merges the caller's headers over its own
    headers.insert(AUTHORIZATION, authorization_header);
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    insert_organization(&mut headers, deployment.organization.as_deref())?;
    insert_headers(&mut headers, supplied)?;
    Ok(headers)
}

pub fn embedding_headers(
    deployment: &Deployment,
    extra: Option<&Headers>,
) -> Result<HeaderMap, OpenAiProviderError> {
    let authorization = deployment
        .credential()
        .ok_or(OpenAiProviderError::Operation(
            OperationError::MissingApiKey,
        ))?;
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        authorization_value(authorization.expose_secret())?,
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    insert_headers(&mut headers, extra)?;
    Ok(headers)
}

fn insert_organization(
    headers: &mut HeaderMap,
    organization: Option<&str>,
) -> Result<(), OpenAiProviderError> {
    let Some(organization_id) = organization else {
        return Ok(());
    };
    headers.insert(
        HeaderName::from_static("openai-organization"),
        header_value(organization_id, "openai-organization")?,
    );
    Ok(())
}

fn insert_headers(
    headers: &mut HeaderMap,
    extra: Option<&Headers>,
) -> Result<(), OpenAiProviderError> {
    let Some(extra_headers) = extra else {
        return Ok(());
    };
    for (name, value) in extra_headers {
        let header_name = HeaderName::try_from(name.as_str()).map_err(|source| {
            OpenAiProviderError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        headers.insert(header_name, header_value(value, name)?);
    }
    Ok(())
}

#[must_use]
pub const fn present_text(value: Option<&str>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

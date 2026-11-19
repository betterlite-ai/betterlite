use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, HeaderName, HeaderValue, header};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::PluginError;

// Hop-by-hop headers (RFC 7230), the litellm session cookie and every header user_api_key_auth accepts as a key.
const REQUEST_STRIP: [&str; 13] = [
    "host",
    "connection",
    "transfer-encoding",
    "te",
    "trailers",
    "upgrade",
    "cookie",
    "authorization",
    "api-key",
    "x-api-key",
    "x-goog-api-key",
    "ocp-apim-subscription-key",
    "x-litellm-api-key",
];

// httpx already decompressed and de-chunked the body, and a plugin must not overwrite litellm cookies.
const RESPONSE_STRIP: [&str; 4] = [
    "content-encoding",
    "transfer-encoding",
    "content-length",
    "set-cookie",
];

// general_settings.litellm_key_header_name, read on every request so a reload applies without a restart
pub trait KeyHeaderName: Send + Sync {
    fn litellm_key_header_name(&self) -> Option<String>;
}

pub fn request_headers(
    raw: &HeaderMap,
    custom_key_header: Option<&str>,
    plugin_key: Option<&SecretString>,
    user_id: Option<&str>,
    user_role: Option<&str>,
) -> Result<HeaderMap, PluginError> {
    let mut forwarded = copy_forwarded(raw, custom_key_header);
    insert_plugin_key(&mut forwarded, plugin_key)?;
    insert_identity(&mut forwarded, user_id, user_role)?;
    Ok(forwarded)
}

fn copy_forwarded(raw: &HeaderMap, custom_key_header: Option<&str>) -> HeaderMap {
    let mut forwarded = HeaderMap::new();
    for (name, value) in raw {
        let configured =
            custom_key_header.is_some_and(|custom| name.as_str().eq_ignore_ascii_case(custom));
        let known = REQUEST_STRIP
            .iter()
            .any(|stripped| name.as_str().eq_ignore_ascii_case(stripped));
        if configured || known {
            continue;
        }
        forwarded.insert(name.clone(), value.clone());
    }
    forwarded
}

fn insert_plugin_key(
    forwarded: &mut HeaderMap,
    plugin_key: Option<&SecretString>,
) -> Result<(), PluginError> {
    let Some(key) = plugin_key else {
        return Ok(());
    };
    let bearer = format!("Bearer {}", key.expose_secret());
    let value =
        HeaderValue::from_str(bearer.as_str()).map_err(|_error| PluginError::HeaderValue {
            name: "authorization",
        })?;
    forwarded.insert(AUTHORIZATION, value);
    Ok(())
}

fn insert_identity(
    forwarded: &mut HeaderMap,
    user_id: Option<&str>,
    user_role: Option<&str>,
) -> Result<(), PluginError> {
    if let Some(user_id) = user_id {
        let value = HeaderValue::from_str(user_id).map_err(|_error| PluginError::HeaderValue {
            name: "x-litellm-user-id",
        })?;
        forwarded.insert(HeaderName::from_static("x-litellm-user-id"), value);
    }
    if let Some(user_role) = user_role {
        let value =
            HeaderValue::from_str(user_role).map_err(|_error| PluginError::HeaderValue {
                name: "x-litellm-user-role",
            })?;
        forwarded.insert(HeaderName::from_static("x-litellm-user-role"), value);
    }
    Ok(())
}

#[must_use]
pub fn response_headers(raw: &HeaderMap) -> HeaderMap {
    let mut safe = HeaderMap::new();
    for (name, value) in raw {
        let known = RESPONSE_STRIP
            .iter()
            .any(|stripped| name.as_str().eq_ignore_ascii_case(stripped));
        if known {
            continue;
        }
        safe.insert(name.clone(), value.clone());
    }
    // set last so a plugin's own copy of either header cannot win
    safe.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox"),
    );
    safe.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    safe
}

mod tests;

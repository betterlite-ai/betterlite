mod tests;

use betterlite_mcp::credential_contracts::auth_config::oauth::ClientCredentialsConfig;
use betterlite_mcp::credential_contracts::errors::{CredError, CredResult};
use betterlite_mcp::credential_contracts::token_endpoint_auth::{
    TokenEndpointClientAuth, build_token_client_auth,
};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use super::token_post::{ExpiresIn, StringField};

const MISSING_PREFIX: &str = "client_credentials config is missing: ";

pub struct PreparedGrant {
    pub token_url: String,
    pub form: Vec<(String, SecretString)>,
    pub headers: Vec<(String, SecretString)>,
    pub identity_key: String,
}

pub fn prepare_grant(config: &ClientCredentialsConfig) -> CredResult<PreparedGrant> {
    let token_url = config
        .token_url
        .as_deref()
        .filter(|value| !value.is_empty());
    let client_id = config
        .client_id
        .as_deref()
        .filter(|value| !value.is_empty());
    let client_secret = config
        .client_secret
        .as_ref()
        .filter(|value| !value.expose_secret().is_empty());
    let present = [
        ("client_id", client_id.is_some()),
        ("client_secret", client_secret.is_some()),
        ("token_url", token_url.is_some()),
    ];
    let (Some(token_url), Some(client_id), Some(client_secret)) =
        (token_url, client_id, client_secret)
    else {
        let missing: Vec<&str> = present
            .iter()
            .filter(|(_, found)| !found)
            .map(|(name, _)| *name)
            .collect();
        return Err(CredError::Misconfigured(format!(
            "{MISSING_PREFIX}{}",
            missing.join(", ")
        )));
    };
    let TokenEndpointClientAuth {
        headers: auth_headers,
        body: auth_body,
    } = build_token_client_auth(
        config.token_endpoint_auth_method,
        Some(client_id),
        Some(client_secret),
    )
    .map_err(|error| CredError::Misconfigured(error.to_string()))?;
    let mut form: Vec<(String, SecretString)> = Vec::new();
    form.push((
        "grant_type".to_owned(),
        SecretString::from("client_credentials".to_owned()),
    ));
    form.extend(auth_body);
    if !config.scopes.is_empty() {
        form.push((
            "scope".to_owned(),
            SecretString::from(config.scopes.join(" ")),
        ));
    }
    if let Some(audience) = config.audience.as_deref().filter(|value| !value.is_empty()) {
        form.push((
            "audience".to_owned(),
            SecretString::from(audience.to_owned()),
        ));
    }
    if let Some(resource) = config
        .upstream_resource
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        form.push((
            "resource".to_owned(),
            SecretString::from(resource.to_owned()),
        ));
    }
    let headers: Vec<(String, SecretString)> = auth_headers.into_iter().collect();
    Ok(PreparedGrant {
        token_url: token_url.to_owned(),
        form,
        headers,
        identity_key: identity_key(config),
    })
}

// Any rotation of a value that names the client identity yields a new key, so an old token is never served for it.
fn identity_key(config: &ClientCredentialsConfig) -> String {
    let scopes = config.scopes.join(" ");
    let secret = config
        .client_secret
        .as_ref()
        .map_or("", |value| value.expose_secret());
    let method = config
        .token_endpoint_auth_method
        .map_or("", |method| method.as_str());
    let material = [
        config.token_url.as_deref().unwrap_or(""),
        config.client_id.as_deref().unwrap_or(""),
        secret,
        method,
        &scopes,
        config.audience.as_deref().unwrap_or(""),
        config.upstream_resource.as_deref().unwrap_or(""),
    ]
    .join("\n");
    hex::encode(Sha256::digest(material.as_bytes()))
}

pub fn parse_expires_in(raw: Option<&ExpiresIn>) -> Option<i64> {
    match raw? {
        ExpiresIn::Flag(_) => None,
        ExpiresIn::Text(text) => parse_integer(text),
        ExpiresIn::Number(number) => number.as_i64(),
        ExpiresIn::Other(_) => None,
    }
}

// Python's int() on text: surrounding whitespace, one sign, and single underscores between digits.
fn parse_integer(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let negative = trimmed.starts_with('-');
    let digits = trimmed.strip_prefix(['-', '+']).unwrap_or(trimmed);
    let mut seen_digit = false;
    let mut pending_underscore = false;
    for byte in digits.bytes() {
        match byte {
            b'0'..=b'9' => {
                seen_digit = true;
                pending_underscore = false;
            }
            b'_' if seen_digit && !pending_underscore => pending_underscore = true,
            _ => return None,
        }
    }
    if !seen_digit || pending_underscore {
        return None;
    }
    let plain = digits.replace('_', "");
    let sign = if negative { "-" } else { "" };
    format!("{sign}{plain}").parse::<i64>().ok()
}

pub fn parse_granted_scopes(raw: Option<&StringField>) -> Vec<String> {
    match raw {
        Some(StringField::Text(text)) if !text.is_empty() => {
            text.split_whitespace().map(str::to_owned).collect()
        }
        Some(StringField::Text(_)) | Some(StringField::Other(_)) | None => Vec::new(),
    }
}

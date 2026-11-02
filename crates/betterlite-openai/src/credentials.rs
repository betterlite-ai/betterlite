use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use std::net::{Ipv4Addr, Ipv6Addr};
use thiserror::Error;

const OPENAI_API_HOST: &str = "api.openai.com";

#[derive(Debug, Error)]
pub enum CredentialsError {
    #[error("api_base is not a valid URL")]
    InvalidApiBase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataResidency {
    Eu,
    Us,
}

#[derive(Debug, Default)]
pub struct ExplicitCredentials {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
}

#[derive(Debug, Default)]
pub struct LiteLlmGlobals {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub organization: Option<String>,
}

#[derive(Debug)]
pub struct OpenAiCredentials {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
}

pub fn resolve_openai_api_base(
    api_base: Option<&str>,
    litellm_api_base: Option<&str>,
    settings: &OpenAiCredentialsSettings,
) -> String {
    present(api_base)
        .or_else(|| present(litellm_api_base))
        .map_or_else(|| settings.api_base.clone(), str::to_owned)
}

pub fn get_openai_credentials(
    explicit: ExplicitCredentials,
    globals: LiteLlmGlobals,
    settings: &OpenAiCredentialsSettings,
    env_api_key: Option<SecretString>,
) -> OpenAiCredentials {
    let api_base = resolve_openai_api_base(
        explicit.api_base.as_deref(),
        globals.api_base.as_deref(),
        settings,
    );
    let organization = present(explicit.organization.as_deref())
        .or_else(|| present(globals.organization.as_deref()))
        .or_else(|| present(settings.organization.as_deref()))
        .map(str::to_owned);
    let api_key = present_secret(explicit.api_key)
        .or_else(|| present_secret(globals.api_key))
        .or_else(|| present_secret(globals.openai_key))
        .or_else(|| present_secret(env_api_key));
    OpenAiCredentials {
        api_base,
        api_key,
        organization,
    }
}

pub fn is_openai_backed_api_base(api_base: &str) -> Result<bool, CredentialsError> {
    let host = extract_host(api_base)?;
    Ok(host.is_some_and(|name| {
        name == OPENAI_API_HOST || name.ends_with(&format!(".{OPENAI_API_HOST}"))
    }))
}

// Mirrors urlsplit(api_base).hostname: a textual split, no percent-decoding or IDNA mapping.
fn extract_host(api_base: &str) -> Result<Option<String>, CredentialsError> {
    let Some(netloc) = split_netloc(api_base) else {
        return Ok(None);
    };
    if netloc.is_empty() {
        return Ok(None);
    }
    let host = raw_host(netloc)?;
    if host.is_empty() {
        return Ok(None);
    }
    Ok(Some(lowercase_host(host)))
}

fn split_netloc(api_base: &str) -> Option<&str> {
    let after_scheme = find_scheme_colon(api_base).map_or(api_base, |colon| {
        let (_, rest) = api_base.split_at(colon.saturating_add(1));
        rest
    });
    let rest = after_scheme.strip_prefix("//")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (netloc, _) = rest.split_at(end);
    Some(netloc)
}

fn find_scheme_colon(api_base: &str) -> Option<usize> {
    let colon = api_base.find(':')?;
    if colon == 0 {
        return None;
    }
    let (prefix, _) = api_base.split_at(colon);
    let mut chars = prefix.chars();
    if !chars.next()?.is_ascii_alphabetic() {
        return None;
    }
    let is_scheme = prefix
        .chars()
        .all(|letter| letter.is_ascii_alphanumeric() || matches!(letter, '+' | '-' | '.'));
    is_scheme.then_some(colon)
}

fn raw_host(netloc: &str) -> Result<&str, CredentialsError> {
    let has_open = netloc.contains('[');
    let has_close = netloc.contains(']');
    if has_open != has_close {
        return Err(CredentialsError::InvalidApiBase);
    }
    let hostinfo = netloc.rsplit_once('@').map_or(netloc, |(_, after)| after);
    if !has_open {
        return Ok(hostinfo
            .split_once(':')
            .map_or(hostinfo, |(host, _port)| host));
    }
    let (before, bracketed) = hostinfo
        .split_once('[')
        .ok_or(CredentialsError::InvalidApiBase)?;
    if !before.is_empty() {
        return Err(CredentialsError::InvalidApiBase);
    }
    let (hostname, after) = bracketed
        .split_once(']')
        .ok_or(CredentialsError::InvalidApiBase)?;
    if !after.is_empty() && !after.starts_with(':') {
        return Err(CredentialsError::InvalidApiBase);
    }
    validate_bracketed_host(hostname)?;
    Ok(hostname)
}

// _check_bracketed_host: an IPvFuture (v<hex>.<rest>) or non-IPv4 address; a bracketed IPv4 literal is rejected.
fn validate_bracketed_host(hostname: &str) -> Result<(), CredentialsError> {
    if let Some(future) = hostname.strip_prefix('v') {
        let mut parts = future.splitn(2, '.');
        let version = parts.next().unwrap_or("");
        let rest = parts.next();
        let valid_future = !version.is_empty()
            && version.bytes().all(|byte| byte.is_ascii_hexdigit())
            && rest.is_some_and(|text| !text.is_empty());
        return valid_future
            .then_some(())
            .ok_or(CredentialsError::InvalidApiBase);
    }
    if hostname.parse::<Ipv4Addr>().is_ok() {
        return Err(CredentialsError::InvalidApiBase);
    }
    hostname
        .parse::<Ipv6Addr>()
        .map(|_ignored| ())
        .map_err(|_ignored| CredentialsError::InvalidApiBase)
}

// hostname(): lowercase everything before a zone-id separator, keeping the zone id as written.
fn lowercase_host(host: &str) -> String {
    let (name, zone) = host.split_once('%').unwrap_or((host, ""));
    if zone.is_empty() {
        name.to_ascii_lowercase()
    } else {
        format!("{}%{zone}", name.to_ascii_lowercase())
    }
}

#[must_use]
pub fn infer_openai_data_residency(
    custom_llm_provider: Option<&str>,
    api_base: Option<&str>,
) -> Option<DataResidency> {
    if custom_llm_provider != Some("openai") {
        return None;
    }
    let host = extract_host(present(api_base)?).ok().flatten()?;
    match host.as_str() {
        "eu.api.openai.com" => Some(DataResidency::Eu),
        "us.api.openai.com" => Some(DataResidency::Us),
        _ => None,
    }
}

const fn present(value: Option<&str>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

fn present_secret(value: Option<SecretString>) -> Option<SecretString> {
    match value {
        Some(secret) if !secret.expose_secret().is_empty() => Some(secret),
        _ => None,
    }
}

mod tests;

use std::net::{Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

use super::error::SearchError;

// LiteLLM takes the first truthy api_base and returns it untouched: no /mcp append and no trim
pub fn complete_url(
    api_base: Option<&str>,
    configured_gateway_url: Option<&str>,
) -> Result<String, SearchError> {
    let explicit = api_base.filter(|value| !value.is_empty());
    let configured = configured_gateway_url.filter(|value| !value.is_empty());
    explicit
        .or(configured)
        .map(str::to_owned)
        .ok_or(SearchError::GatewayUrlMissing)
}

// credentials may travel only over https or to a loopback host, whatever the scheme
#[must_use]
pub fn credential_safe_transport(api_base: &str) -> bool {
    let Ok(parsed) = Url::parse(api_base) else {
        return false;
    };
    parsed.scheme() == "https" || loopback_host(&parsed)
}

fn loopback_host(parsed: &Url) -> bool {
    match parsed.host() {
        Some(Host::Domain(name)) => name == "localhost",
        Some(Host::Ipv4(address)) => address == Ipv4Addr::LOCALHOST,
        Some(Host::Ipv6(address)) => address == Ipv6Addr::LOCALHOST,
        None => false,
    }
}

// the region capture of [a-z0-9-]+\.gateway\.bedrock-agentcore\.([a-z0-9-]+)\.amazonaws\.com
#[must_use]
pub fn gateway_region(api_base: &str) -> Option<String> {
    let parsed = Url::parse(api_base).ok()?;
    let Some(Host::Domain(host)) = parsed.host() else {
        return None;
    };
    region_of_gateway_host(host)
}

fn region_of_gateway_host(host: &str) -> Option<String> {
    let mut segments = host.split('.');
    if !gateway_label(segments.next()?) {
        return None;
    }
    if segments.next()? != "gateway" {
        return None;
    }
    if segments.next()? != "bedrock-agentcore" {
        return None;
    }
    let region = segments.next()?;
    if !gateway_label(region) {
        return None;
    }
    if segments.next()? != "amazonaws" || segments.next()? != "com" {
        return None;
    }
    if segments.next().is_some() {
        return None;
    }
    Some(region.to_owned())
}

fn gateway_label(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

// the custom-host allowlist compares hostnames only, unlike the netloc trust on bearer tokens
#[must_use]
pub fn is_configured_gateway(api_base: &str, configured_gateway_url: Option<&str>) -> bool {
    let Some(configured) = configured_gateway_url.filter(|value| !value.is_empty()) else {
        return false;
    };
    let (Ok(candidate), Ok(trusted)) = (Url::parse(api_base), Url::parse(configured)) else {
        return false;
    };
    match (candidate.host_str(), trusted.host_str()) {
        (Some(candidate), Some(trusted)) => candidate == trusted,
        _ => false,
    }
}

// boto3's session region covers AWS_DEFAULT_REGION and the profile region; there is no default
pub fn signing_region(
    api_base: &str,
    matched_region: Option<&str>,
    session_region: Option<&str>,
) -> Result<String, SearchError> {
    if let Some(region) = matched_region {
        return Ok(region.to_owned());
    }
    if let Some(region) = session_region.filter(|value| !value.is_empty()) {
        return Ok(region.to_owned());
    }
    Err(SearchError::SigningRegion {
        api_base: api_base.to_owned(),
    })
}

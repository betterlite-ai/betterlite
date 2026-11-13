use betterlite_config::tls::Verification;
use fancy_regex::Regex;
use url::{ParseError, Url};

use super::error::AwsEndpointError;

const STS_REGION_PATTERN: &str =
    r"(?:^|\.)sts(?:-fips)?\.([a-z0-9-]+)\.(?:amazonaws\.com(?:\.cn)?|vpce\.amazonaws\.com)";

#[derive(Debug, Clone, Copy)]
pub struct StsSettings<'text> {
    pub endpoint: Option<&'text str>,
    pub aws_region: Option<&'text str>,
    pub aws_default_region: Option<&'text str>,
    pub configured_region: Option<&'text str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StsClientKwargs {
    pub verify: Verification,
    pub endpoint_url: Option<String>,
    pub region_name: Option<String>,
}

pub fn sts_region_from_endpoint(
    endpoint: Option<&str>,
) -> Result<Option<String>, AwsEndpointError> {
    let Some(value) = endpoint.filter(|candidate| !candidate.is_empty()) else {
        return Ok(None);
    };
    let host = hostname(value)?;
    sts_region_in_host(&host)
}

pub fn resolve_sts_region(settings: &StsSettings<'_>) -> Result<Option<String>, AwsEndpointError> {
    if let Some(region) = sts_region_from_endpoint(settings.endpoint)? {
        return Ok(Some(region));
    }
    for candidate in [settings.aws_region, settings.aws_default_region] {
        if let Some(region) = candidate.filter(|value| !value.is_empty()) {
            return Ok(Some(region.to_owned()));
        }
    }
    Ok(settings.configured_region.map(str::to_owned))
}

pub fn sts_client_kwargs(
    settings: &StsSettings<'_>,
    verify: Verification,
) -> Result<StsClientKwargs, AwsEndpointError> {
    Ok(StsClientKwargs {
        verify,
        endpoint_url: settings.endpoint.map(str::to_owned),
        region_name: resolve_sts_region(settings)?,
    })
}

fn hostname(endpoint: &str) -> Result<String, AwsEndpointError> {
    let parsed = if endpoint.starts_with("//") {
        Url::parse(&format!("https:{endpoint}"))
    } else {
        Url::parse(endpoint)
    };
    match parsed {
        Ok(url) => Ok(url.host_str().unwrap_or_default().to_lowercase()),
        Err(ParseError::RelativeUrlWithoutBase) => Ok(String::new()),
        Err(source) => Err(AwsEndpointError::Endpoint { source }),
    }
}

// LiteLLM's _STS_REGION_FROM_ENDPOINT_PATTERN searches the lowercased host without an end anchor
fn sts_region_in_host(host: &str) -> Result<Option<String>, AwsEndpointError> {
    let pattern = Regex::new(STS_REGION_PATTERN)?;
    let captures = pattern.captures(host)?;
    let Some(matched) = captures else {
        return Ok(None);
    };
    Ok(matched.get(1).map(|region| region.as_str().to_owned()))
}

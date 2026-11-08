use betterlite_config::tls::Verification;
use url::{ParseError, Url};

use super::error::AwsEndpointError;

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
    Ok(sts_region_in_host(&host))
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

// LiteLLM's _STS_REGION_FROM_ENDPOINT_PATTERN, matched anywhere in the lowercase host
fn sts_region_in_host(host: &str) -> Option<String> {
    for (start, _) in host.match_indices("sts") {
        let boundary = start == 0
            || host
                .get(..start)
                .is_some_and(|prefix| prefix.ends_with('.'));
        if !boundary {
            continue;
        }
        let Some(raw_tail) = host.get(start..).and_then(|rest| rest.strip_prefix("sts")) else {
            continue;
        };
        let tail = raw_tail.strip_prefix("-fips").unwrap_or(raw_tail);
        let Some(after_dot) = tail.strip_prefix('.') else {
            continue;
        };
        let length = after_dot
            .bytes()
            .take_while(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
            .count();
        if length == 0 {
            continue;
        }
        let Some(region) = after_dot.get(..length) else {
            continue;
        };
        let Some(suffix) = after_dot.get(length..) else {
            continue;
        };
        if suffix.starts_with(".amazonaws.com") || suffix.starts_with(".vpce.amazonaws.com") {
            return Some(region.to_owned());
        }
    }
    None
}

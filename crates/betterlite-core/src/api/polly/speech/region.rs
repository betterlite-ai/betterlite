use super::error::SpeechError;

// B591's non_llm_region_name fallback; Polly's own DEFAULT_REGION is the URL and signing default
pub const FALLBACK_REGION: &str = "us-west-2";

#[derive(Clone, Copy)]
pub struct RegionEnvironment<'text> {
    pub region_name: Option<&'text str>,
    pub region: Option<&'text str>,
}

// dispatch takes a truthy kwargs region, then a non-null optional region, and only a null one asks the environment
pub fn dispatch_region(
    kwargs_region: Option<&str>,
    optional_region: Option<&str>,
    environment: RegionEnvironment<'_>,
) -> Result<String, SpeechError> {
    if let Some(region) = kwargs_region.filter(|value| !value.is_empty()) {
        return Ok(region.to_owned());
    }
    if let Some(region) = optional_region {
        return Ok(region.to_owned());
    }
    resolved_region(environment)
}

// AWS_REGION overrides AWS_REGION_NAME, and the selected name is validated only on this path
fn resolved_region(environment: RegionEnvironment<'_>) -> Result<String, SpeechError> {
    let selected = environment
        .region
        .or(environment.region_name)
        .unwrap_or(FALLBACK_REGION);
    validate(selected)?;
    Ok(selected.to_owned())
}

// LiteLLM's _VALID_AWS_REGION_PATTERN is \A[a-z0-9-]+\Z
fn validate(region: &str) -> Result<(), SpeechError> {
    let valid = !region.is_empty()
        && region
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid {
        return Ok(());
    }
    Err(SpeechError::InvalidRegion {
        region: region.to_owned(),
    })
}

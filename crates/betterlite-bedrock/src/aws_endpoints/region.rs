use betterlite_context::aws_partition::contains_bedrock_arn;

use super::error::AwsEndpointError;

pub const DEFAULT_AWS_REGION: &str = "us-west-2";

#[derive(Debug, Clone, Copy)]
pub struct LlmRegion<'text> {
    pub explicit: Option<&'text str>,
    pub model: Option<&'text str>,
    pub model_id: Option<&'text str>,
    pub environment_region_name: Option<&'text str>,
    pub environment_region: Option<&'text str>,
    pub session_region: Option<&'text str>,
}

#[must_use]
pub fn region_from_model_arn(model: Option<&str>) -> Option<String> {
    let value = model?;
    if !contains_bedrock_arn(value) {
        return None;
    }
    let region = value.split(':').nth(3)?;
    is_region(region).then(|| region.to_owned())
}

pub fn validate_region(region: Option<&str>) -> Result<(), AwsEndpointError> {
    let Some(value) = region else {
        return Ok(());
    };
    if is_region(value) {
        return Ok(());
    }
    Err(AwsEndpointError::Region {
        region: value.to_owned(),
    })
}

// LiteLLM's _VALID_AWS_REGION_PATTERN is \A[a-z0-9-]+\Z
fn is_region(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub fn llm_region_name(input: LlmRegion<'_>) -> Result<String, AwsEndpointError> {
    validate_region(input.explicit)?;
    let selected = input
        .explicit
        .map(str::to_owned)
        .or_else(|| region_from_model_arn(input.model_id.or(input.model)))
        .or_else(|| input.environment_region_name.map(str::to_owned))
        .or_else(|| input.environment_region.map(str::to_owned))
        .unwrap_or_else(|| {
            input
                .session_region
                .filter(|value| !value.is_empty())
                .unwrap_or(DEFAULT_AWS_REGION)
                .to_owned()
        });
    validate_region(Some(&selected))?;
    Ok(selected)
}

pub fn non_llm_region_name(
    explicit: Option<&str>,
    environment_region_name: Option<&str>,
    environment_region: Option<&str>,
) -> Result<String, AwsEndpointError> {
    validate_region(explicit)?;
    let selected = explicit
        .map(str::to_owned)
        .or_else(|| environment_region.map(str::to_owned))
        .or_else(|| environment_region_name.map(str::to_owned))
        .unwrap_or_else(|| DEFAULT_AWS_REGION.to_owned());
    validate_region(Some(&selected))?;
    Ok(selected)
}

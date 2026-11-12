use betterlite_config::sections::bedrock_aws_endpoints::Settings as AwsEndpointSettings;
use betterlite_config::sections::claude_platform_inputs::PlatformEnvironment;

use super::error::PlatformInputError;
use super::{CLAUDE_PLATFORM_SERVICE_NAME, Candidate, optional_candidate, select_matching};
use crate::aws_endpoints::region::validate_region;
use crate::model_routes::ParamMap;

pub struct UrlInputs<'source> {
    pub api_base: Option<&'source str>,
    pub litellm_api_base: Option<&'source str>,
    pub optional_params: &'source ParamMap,
}

pub fn resolve_region(
    optional_params: &ParamMap,
    environment: &AwsEndpointSettings,
) -> Result<String, PlatformInputError> {
    let selected = select_matching(
        [
            optional_candidate(optional_params, "aws_region_name"),
            environment.aws_region_name.as_deref().map(Candidate::Text),
            environment.aws_region.as_deref().map(Candidate::Text),
            environment
                .aws_default_region
                .as_deref()
                .map(Candidate::Text),
        ],
        Candidate::is_effective,
    )
    .ok_or(PlatformInputError::MissingRegion)?;
    let region = selected.render()?;
    validate_region(Some(&region))?;
    Ok(region)
}

pub fn complete_url(
    inputs: &UrlInputs<'_>,
    environment: &PlatformEnvironment,
    aws: &AwsEndpointSettings,
) -> Result<String, PlatformInputError> {
    let selected = select_matching(
        [
            inputs.api_base,
            inputs.litellm_api_base,
            environment.anthropic_aws_base_url.as_deref(),
            environment.anthropic_aws_api_base.as_deref(),
        ],
        |text| !text.is_empty(),
    );
    if let Some(base) = selected {
        return Ok(finish_url(base));
    }
    let region = resolve_region(inputs.optional_params, aws)?;
    Ok(finish_url(&format!(
        "https://{CLAUDE_PLATFORM_SERVICE_NAME}.{region}.api.aws"
    )))
}

fn finish_url(base: &str) -> String {
    if base.ends_with("/v1/messages") {
        base.to_owned()
    } else {
        format!("{}/v1/messages", base.trim_end_matches('/'))
    }
}

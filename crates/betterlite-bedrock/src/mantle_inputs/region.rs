use betterlite_context::bedrock_model::regions::AmazonBedrockGlobalConfig;
use fancy_regex::Regex;

use crate::aws_endpoints::region::validate_region;

use super::error::MantleInputError;
use super::records::MantleGlobals;

pub const DEFAULT_REGION: &str = "us-east-1";

// Python's `$` also matches before one trailing newline, so the lookahead spells both ends out.
const MANTLE_HOST_PATTERN: &str = r"(?i)^https?://bedrock-mantle\.([^/.]+)\.api\.aws(?=/|\n?\z)";

#[must_use]
pub fn split_region_prefix(model: &str) -> (Option<&str>, &str) {
    let Some((head, tail)) = model.split_once('/') else {
        return (None, model);
    };
    if AmazonBedrockGlobalConfig.get_all_regions().contains(&head) {
        (Some(head), tail)
    } else {
        (None, model)
    }
}

pub fn resolve_region(
    aws_region_name: Option<&str>,
    api_base: Option<&str>,
    globals: &MantleGlobals,
) -> Result<String, MantleInputError> {
    if let Some(region) = aws_region_name.filter(|value| !value.is_empty()) {
        validate_region(Some(region))?;
        return Ok(region.to_owned());
    }
    let base = api_base.filter(|value| !value.is_empty()).or_else(|| {
        globals
            .mantle_api_base
            .as_deref()
            .filter(|value| !value.is_empty())
    });
    if let Some(region) = base.and_then(region_from_host) {
        return Ok(region);
    }
    Ok(fallback_region(globals))
}

pub(crate) fn region_from_host(base: &str) -> Option<String> {
    let pattern = Regex::new(MANTLE_HOST_PATTERN).ok()?;
    let captures = pattern.captures(base.trim_end_matches('/')).ok()??;
    captures.get(1).map(|region| region.as_str().to_owned())
}

fn fallback_region(globals: &MantleGlobals) -> String {
    [
        globals.mantle_region.as_deref(),
        globals.aws_region_name.as_deref(),
        globals.aws_region.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find(|value| !value.is_empty())
    .unwrap_or(DEFAULT_REGION)
    .to_owned()
}

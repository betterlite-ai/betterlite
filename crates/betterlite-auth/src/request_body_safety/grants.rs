use betterlite_config::sections::request_body_safety::ClientsideParamGrant;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use pcre2::bytes::RegexBuilder;

use super::{ModelGrantLookup, ModelGrants, SafetyError};

const API_BASE: &str = "api_base";

// LiteLLM's _allow_model_level_clientside_configurable_parameters: the full group name first, then the
// provider prefix only while B174's provider list holds it, and no group answers with a denial.
pub fn model_level_allows(
    model: &str,
    param: &str,
    value: &Json,
    lookup: &dyn ModelGrantLookup,
) -> Result<bool, SafetyError> {
    let mut grants = lookup.grants(model);
    if matches!(grants, ModelGrants::Absent)
        && let Some(prefix) = provider_prefix(model)
    {
        grants = lookup.grants(prefix);
    }
    match grants {
        ModelGrants::Granted(items) => is_param_allowed(param, value, items),
        ModelGrants::Unset | ModelGrants::Absent => Ok(false),
    }
}

fn provider_prefix(model: &str) -> Option<&str> {
    let (prefix, _rest) = model.split_once('/')?;
    if prefix.parse::<ProviderId>().is_err() {
        return None;
    }
    Some(prefix)
}

// LiteLLM's _is_param_allowed: a string entry grants its own parameter exactly, and a mapping entry is
// considered only for api_base, where its api_base entry is matched before the value is compared for equality.
pub fn is_param_allowed(
    param: &str,
    value: &Json,
    items: &[ClientsideParamGrant],
) -> Result<bool, SafetyError> {
    for item in items {
        match item {
            ClientsideParamGrant::Exact(name) => {
                if param == name {
                    return Ok(true);
                }
            }
            ClientsideParamGrant::Matcher(fields) => {
                if param != API_BASE {
                    continue;
                }
                let Some(candidate) = fields.get(API_BASE) else {
                    return Err(SafetyError::MatcherMissing);
                };
                let Json::Text(pattern) = candidate else {
                    return Err(SafetyError::MatcherType);
                };
                let Json::Text(text) = value else {
                    return Err(SafetyError::MatcherType);
                };
                if check_regex_or_str_match(text, pattern)? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

// Python's re.match starts at the subject start, and the bound keeps a caller pattern from exhausting the matcher
pub fn check_regex_or_str_match(value: &str, pattern: &str) -> Result<bool, SafetyError> {
    let expression = format!("(*LIMIT_MATCH=1000000)(*LIMIT_DEPTH=256)\\A(?:{pattern})");
    let engine = RegexBuilder::new()
        .utf(true)
        .ucp(true)
        .build(&expression)
        .map_err(SafetyError::MatcherRegex)?;
    if engine
        .is_match(value.as_bytes())
        .map_err(SafetyError::MatcherRegex)?
    {
        return Ok(true);
    }
    Ok(pattern == value)
}

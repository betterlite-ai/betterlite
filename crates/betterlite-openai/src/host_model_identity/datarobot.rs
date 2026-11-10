use betterlite_models::provider_defaults_a::url::{parse, unparse};

use super::error::HostModelIdentityError;

const LLMGW_PATH: &str = "/genai/llmgw/chat/completions";
const LLMGW_SUFFIX: &str = "/api/v2/genai/llmgw/chat/completions";

// DataRobotConfig._resolve_api_base edits only the path of the endpoint the caller configured
pub fn transform_api_base(api_base: &str) -> Result<String, HostModelIdentityError> {
    let mut parts = parse(api_base).map_err(|source| HostModelIdentityError::ApiBase {
        api_base: api_base.to_owned(),
        source,
    })?;
    let path = &parts.path;
    let mut resolved = if path.is_empty() || path == "/" {
        format!("{path}{LLMGW_SUFFIX}")
    } else if path.contains("api/v2")
        && !path.contains("api/v2/deployments")
        && !path.contains(LLMGW_PATH)
    {
        format!("{path}{LLMGW_PATH}")
    } else {
        path.clone()
    };
    if !resolved.ends_with('/') {
        resolved.push('/');
    }
    // one pass, so a longer run of slashes keeps the pairs a single pass leaves behind
    parts.path = resolved.replace("//", "/");
    Ok(unparse(&parts))
}

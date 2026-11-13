use std::collections::BTreeSet;

use betterlite_models::decisions::Json as ParamsJson;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;

use crate::credentials::{AnthropicSettings, OAUTH_BETA_HEADER_NAME, handle_anthropic_oauth};
use crate::feature_flags::FeatureParams;
use crate::feature_flags::beta::user_beta_headers;
use crate::feature_flags::messages::{
    has_output_config, has_thinking_display_updates, has_tool_change, is_cache_control_set,
    is_file_id_used, is_mcp_server_used, is_pdf_used,
};
use crate::feature_flags::tools::{
    computer_tool_version, is_code_execution_tool_used, is_container_with_skills_used,
    is_tool_search_used, is_web_search_tool_used, uses_input_examples, uses_programmatic_calls,
};

use super::request::{HeaderFeature, HeaderFeatures, HeaderRequest, get_anthropic_headers};
use super::{AnthropicHeaders, HeadersError};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HeaderParams {
    pub tools: Option<Vec<Json>>,
    pub mcp_servers: Option<Vec<Json>>,
    pub thinking: Option<Json>,
    pub is_vertex_request: bool,
    pub container: Option<ParamsJson>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HeaderLiteLlmParams {
    pub api_base: Option<String>,
    pub use_bearer_for_custom_base: bool,
}

pub struct EnvironmentRequest<'request> {
    pub model: &'request str,
    pub caller_headers: &'request IndexMap<String, String>,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub litellm_params: &'request HeaderLiteLlmParams,
    pub messages: &'request [Json],
    pub optional_params: &'request HeaderParams,
    // B688 owns is_effort_used; the caller runs it and passes the answer
    pub effort_used: bool,
}

pub fn validate_environment(
    request: &EnvironmentRequest<'_>,
    settings: &AnthropicSettings,
) -> Result<AnthropicHeaders, HeadersError> {
    let api_base = request
        .api_base
        .or(request.litellm_params.api_base.as_deref());
    let mut headers = request.caller_headers.clone();
    let mut api_key = request.api_key.cloned();
    if let Some(oauth_key) = handle_anthropic_oauth(&mut headers, api_key.clone()) {
        api_key = Some(oauth_key);
    }
    let (api_key, auth_token) = resolve_credentials(api_key, request, settings)?;
    let features = detected_features(request)?;
    let user_betas = user_beta_headers(headers.get(OAUTH_BETA_HEADER_NAME).map(String::as_str));
    let generated = get_anthropic_headers(&HeaderRequest {
        api_key: api_key.as_ref(),
        auth_token: auth_token.as_ref(),
        anthropic_version: None,
        api_base,
        use_bearer_for_custom_base: request.litellm_params.use_bearer_for_custom_base,
        user_betas: user_betas.as_deref(),
        features: &features,
    });
    headers.extend(generated.values);
    Ok(AnthropicHeaders { values: headers })
}

fn resolve_credentials(
    api_key: Option<SecretString>,
    request: &EnvironmentRequest<'_>,
    settings: &AnthropicSettings,
) -> Result<(Option<SecretString>, Option<SecretString>), HeadersError> {
    if let Some(resolved) = api_key.or_else(|| settings.api_key.clone()) {
        return Ok((Some(resolved), None));
    }
    if let Some(token) = settings.auth_token.clone() {
        return Ok((None, Some(token)));
    }
    Err(HeadersError::Authentication {
        model: request.model.to_owned(),
    })
}

fn detected_features(request: &EnvironmentRequest<'_>) -> Result<HeaderFeatures, HeadersError> {
    let tools = request.optional_params.tools.as_deref().unwrap_or_default();
    let params = FeatureParams {
        compaction: None,
        container: request.optional_params.container.clone(),
        output_config: None,
        reasoning_effort: None,
    };
    Ok(HeaderFeatures {
        features: detected_features_set(request, tools, &params)?,
        computer_tool_used: tools.iter().find_map(computer_tool_version),
        is_vertex_request: request.optional_params.is_vertex_request,
        web_search_tool_used: is_web_search_tool_used(tools),
    })
}

fn detected_features_set(
    request: &EnvironmentRequest<'_>,
    tools: &[Json],
    params: &FeatureParams,
) -> Result<BTreeSet<HeaderFeature>, HeadersError> {
    let mut features = BTreeSet::new();
    let detected = [
        (
            HeaderFeature::PromptCaching,
            is_cache_control_set(request.messages),
        ),
        (HeaderFeature::Pdf, is_pdf_used(request.messages)),
        (HeaderFeature::FileId, is_file_id_used(request.messages)?),
        (
            HeaderFeature::McpServer,
            is_mcp_server_used(request.optional_params.mcp_servers.as_deref()),
        ),
        (HeaderFeature::ToolSearch, is_tool_search_used(tools)),
        (
            HeaderFeature::ProgrammaticToolCalling,
            uses_programmatic_calls(tools),
        ),
        (HeaderFeature::InputExamples, uses_input_examples(tools)),
        (
            HeaderFeature::CodeExecution,
            is_code_execution_tool_used(tools),
        ),
        (HeaderFeature::Skills, is_container_with_skills_used(params)),
        (
            HeaderFeature::OutputConfig,
            has_output_config(request.messages),
        ),
        (
            HeaderFeature::ThinkingDisplayUpdates,
            has_thinking_display_updates(request.optional_params.thinking.as_ref()),
        ),
        (
            HeaderFeature::ToolChanges,
            has_tool_change(request.messages),
        ),
        (HeaderFeature::Effort, request.effort_used),
    ];
    for (feature, used) in detected {
        if used {
            features.insert(feature);
        }
    }
    Ok(features)
}

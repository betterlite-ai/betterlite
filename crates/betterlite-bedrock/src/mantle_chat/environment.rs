use indexmap::IndexMap;
use secrecy::SecretString;

use crate::invoke_primitives::config::Params;
use crate::invoke_primitives::environment::{EnvironmentError, validate_environment};
use crate::messages::transport::validate_messages_environment;
use crate::request_policy::{BedrockRequestPolicy, Params as MetadataParams, RequestValue};

use super::{ANTHROPIC_VERSION_HEADER, ANTHROPIC_WORKSPACE_HEADER, DEFAULT_ANTHROPIC_API_VERSION};

const PROJECT_ID_FIELD: &str = "aws_bedrock_project_id";

// AmazonMantleConfig.validate_environment: the parent's merge, then a truthy project ID overwrites anthropic-workspace.
pub fn chat_headers(
    headers: &IndexMap<String, String>,
    optional_params: &mut Params,
    litellm_params: &MetadataParams,
    policy: Option<&BedrockRequestPolicy>,
) -> Result<IndexMap<String, String>, EnvironmentError> {
    let mut merged = validate_environment(headers, optional_params, litellm_params, policy)?;
    if let Some(project_id) = project_id(litellm_params) {
        merged.insert(ANTHROPIC_WORKSPACE_HEADER.to_owned(), project_id.to_owned());
    }
    Ok(merged)
}

// AmazonMantleMessagesConfig.validate_anthropic_messages_environment: the parent's merge, then the project ID and the default anthropic-version.
pub fn messages_headers(
    headers: &IndexMap<String, String>,
    litellm_params: &MetadataParams,
    policy: Option<&BedrockRequestPolicy>,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
) -> Result<(IndexMap<String, String>, Option<String>), EnvironmentError> {
    let (mut merged, resolved_api_base) =
        validate_messages_environment(headers, litellm_params, policy, api_base, api_key)?;
    if let Some(project_id) = project_id(litellm_params) {
        merged.insert(ANTHROPIC_WORKSPACE_HEADER.to_owned(), project_id.to_owned());
    }
    let has_version = merged
        .keys()
        .any(|name| name.eq_ignore_ascii_case(ANTHROPIC_VERSION_HEADER));
    if !has_version {
        merged.insert(
            ANTHROPIC_VERSION_HEADER.to_owned(),
            DEFAULT_ANTHROPIC_API_VERSION.to_owned(),
        );
    }
    Ok((merged, resolved_api_base))
}

// LiteLLM's `if project_id:`: an empty value is falsey and leaves the headers untouched.
fn project_id(litellm_params: &MetadataParams) -> Option<&str> {
    match litellm_params.get(PROJECT_ID_FIELD) {
        Some(RequestValue::Text(value)) if !value.is_empty() => Some(value),
        Some(
            RequestValue::Text(_)
            | RequestValue::Secret(_)
            | RequestValue::Mapping(_)
            | RequestValue::Raw(_),
        )
        | None => None,
    }
}

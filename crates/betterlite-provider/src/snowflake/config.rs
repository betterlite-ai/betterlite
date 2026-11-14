use betterlite_prompts::arguments::json::{Json, Object};

use super::SnowflakeError;
use crate::api::file_contracts::config::Headers;

const CHAT_PARAMS: [&str; 7] = [
    "temperature",
    "max_tokens",
    "max_completion_tokens",
    "top_p",
    "stream",
    "tools",
    "tool_choice",
];
const EMBEDDING_PARAMS: [&str; 7] = [
    "temperature",
    "max_tokens",
    "top_p",
    "stream",
    "response_format",
    "tools",
    "tool_choice",
];
const THINKING: &str = "thinking";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const API_SUFFIX: &str = "/api/v2";
const CLAUDE_PREFIXES: [&str; 2] = ["claude-", "claude_"];
const SNOWFLAKE_PREFIX: &str = "snowflake/";
const PAT_PREFIX: &str = "pat/";
const JWT_TOKEN_TYPE: &str = "KEYPAIR_JWT";
const PAT_TOKEN_TYPE: &str = "PROGRAMMATIC_ACCESS_TOKEN";

#[must_use]
pub fn is_claude_model(model: &str) -> bool {
    let lower = model.to_lowercase();
    let name = lower.strip_prefix(SNOWFLAKE_PREFIX).unwrap_or(&lower);
    CLAUDE_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

#[must_use]
pub fn get_supported_openai_params(model: &str) -> Vec<&'static str> {
    let mut params = CHAT_PARAMS.to_vec();
    if is_claude_model(model) {
        params.push(THINKING);
    }
    params
}

// LiteLLM's SnowflakeBaseConfig list, which the embedding config inherits
#[must_use]
pub fn get_embedding_supported_openai_params() -> Vec<&'static str> {
    EMBEDDING_PARAMS.to_vec()
}

pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object, model: &str) {
    let supported = get_supported_openai_params(model);
    copy_supported(non_default_params, optional_params, &supported);
}

pub fn map_embedding_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    let supported = get_embedding_supported_openai_params();
    copy_supported(non_default_params, optional_params, &supported);
}

fn copy_supported(non_default_params: &Object, optional_params: &mut Object, supported: &[&str]) {
    for (name, value) in non_default_params {
        if supported.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

// A given api_base skips validation and leaves account_id in optional_params for the body
pub fn get_api_base(
    api_base: Option<&str>,
    optional_params: &mut Object,
    account_id_env: Option<&str>,
) -> Result<String, SnowflakeError> {
    let Some(given) = api_base.filter(|base| !base.is_empty()) else {
        let account_id = resolve_account_id(optional_params, account_id_env)?;
        let account_id = account_id.ok_or(SnowflakeError::MissingAccountId)?;
        if !valid_account_id(&account_id) {
            return Err(SnowflakeError::InvalidAccountId);
        }
        let base = format!("https://{account_id}.snowflakecomputing.com{API_SUFFIX}");
        return Ok(base);
    };
    Ok(with_api_suffix(given))
}

fn resolve_account_id(
    optional_params: &mut Object,
    account_id_env: Option<&str>,
) -> Result<Option<String>, SnowflakeError> {
    match optional_params.shift_remove("account_id") {
        None => Ok(account_id_env.map(str::to_owned)),
        Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(SnowflakeError::AccountIdType)
        }
    }
}

// Python's re.match with "$" also accepts one newline after an otherwise valid id
fn valid_account_id(account_id: &str) -> bool {
    let body = account_id.strip_suffix('\n').unwrap_or(account_id);
    !body.is_empty()
        && !body.contains('\n')
        && body.chars().all(|candidate| {
            candidate.is_ascii_alphanumeric() || candidate == '_' || candidate == '-'
        })
}

fn with_api_suffix(api_base: &str) -> String {
    let trimmed = api_base.trim_end_matches('/');
    if trimmed.ends_with(API_SUFFIX) {
        trimmed.to_owned()
    } else {
        format!("{trimmed}{API_SUFFIX}")
    }
}

pub fn get_complete_url(
    api_base: Option<&str>,
    model: &str,
    optional_params: &mut Object,
    account_id_env: Option<&str>,
) -> Result<String, SnowflakeError> {
    let base = get_api_base(api_base, optional_params, account_id_env)?;
    let path = if is_claude_model(model) {
        "/cortex/v1/messages"
    } else {
        "/cortex/v1/chat/completions"
    };
    Ok(format!("{base}{path}"))
}

pub fn get_embed_complete_url(
    api_base: Option<&str>,
    optional_params: &mut Object,
    account_id_env: Option<&str>,
) -> Result<String, SnowflakeError> {
    let base = get_api_base(api_base, optional_params, account_id_env)?;
    Ok(format!("{base}/cortex/inference:embed"))
}

pub fn validate_environment(
    headers: &mut Headers,
    model: &str,
    api_key: Option<&str>,
) -> Result<(), SnowflakeError> {
    let key = api_key.ok_or(SnowflakeError::MissingJwtKey)?;
    let (key, auth_type) = key
        .strip_prefix(PAT_PREFIX)
        .map_or((key, JWT_TOKEN_TYPE), |token| (token, PAT_TOKEN_TYPE));
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    headers.insert("Accept".to_owned(), "application/json".to_owned());
    headers.insert("Authorization".to_owned(), format!("Bearer {key}"));
    headers.insert(
        "X-Snowflake-Authorization-Token-Type".to_owned(),
        auth_type.to_owned(),
    );
    if is_claude_model(model) {
        headers.insert("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned());
    }
    Ok(())
}

#[must_use]
pub fn get_openai_compatible_provider_info(
    api_base: Option<&str>,
    api_key: Option<&str>,
    jwt_env: Option<&str>,
) -> (Option<String>, Option<String>) {
    let dynamic_key = api_key
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .or_else(|| jwt_env.map(str::to_owned));
    (api_base.map(str::to_owned), dynamic_key)
}

#[cfg(test)]
mod tests;

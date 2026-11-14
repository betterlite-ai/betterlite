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
pub const EMBEDDING_PARAMS: [&str; 7] = [
    "temperature",
    "max_tokens",
    "top_p",
    "stream",
    "response_format",
    "tools",
    "tool_choice",
];
const API_SUFFIX: &str = "/api/v2";

pub enum Endpoint<'model> {
    Chat(&'model str),
    Embedding,
}

#[must_use]
pub fn is_claude_model(model: &str) -> bool {
    let lower = model.to_lowercase();
    let name = lower.strip_prefix("snowflake/").unwrap_or(&lower);
    name.starts_with("claude-") || name.starts_with("claude_")
}

#[must_use]
pub fn get_supported_openai_params(model: &str) -> Vec<&'static str> {
    let mut params = CHAT_PARAMS.to_vec();
    params.extend(is_claude_model(model).then_some("thinking"));
    params
}

pub fn map_openai_params(
    non_default_params: &Object,
    optional_params: &mut Object,
    supported: &[&str],
) {
    for (name, value) in non_default_params {
        if supported.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

// Python's re.match with "$" also accepts one newline after an otherwise valid id
fn valid_account_id(account_id: &str) -> bool {
    let body = account_id.strip_suffix('\n').unwrap_or(account_id);
    !body.is_empty()
        && !body.contains('\n')
        && body
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || letter == '_' || letter == '-')
}

fn with_api_suffix(api_base: &str) -> String {
    let trimmed = api_base.trim_end_matches('/');
    if trimmed.ends_with(API_SUFFIX) {
        return trimmed.to_owned();
    }
    format!("{trimmed}{API_SUFFIX}")
}

// A given api_base skips validation and leaves account_id in optional_params for the body
pub fn get_complete_url(
    api_base: Option<&str>,
    endpoint: &Endpoint<'_>,
    params: &mut Object,
    account_id_env: Option<&str>,
) -> Result<String, SnowflakeError> {
    let path = match endpoint {
        Endpoint::Chat(model) if is_claude_model(model) => "/cortex/v1/messages",
        Endpoint::Chat(_) => "/cortex/v1/chat/completions",
        Endpoint::Embedding => "/cortex/inference:embed",
    };
    if let Some(given) = api_base.filter(|base| !base.is_empty()) {
        return Ok(format!("{}{path}", with_api_suffix(given)));
    }
    let resolved = match params.shift_remove("account_id") {
        Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(SnowflakeError::AccountIdType);
        }
        None => account_id_env.map(str::to_owned),
    };
    let account_id = resolved.ok_or(SnowflakeError::MissingAccountId)?;
    if !valid_account_id(&account_id) {
        return Err(SnowflakeError::InvalidAccountId);
    }
    Ok(format!(
        "https://{account_id}.snowflakecomputing.com{API_SUFFIX}{path}"
    ))
}

pub fn validate_environment(
    headers: &mut Headers,
    model: &str,
    api_key: Option<&str>,
) -> Result<(), SnowflakeError> {
    let key = api_key.ok_or(SnowflakeError::MissingJwtKey)?;
    let (token, auth_type) = key
        .strip_prefix("pat/")
        .map_or((key, "KEYPAIR_JWT"), |token| {
            (token, "PROGRAMMATIC_ACCESS_TOKEN")
        });
    headers.extend([
        ("Content-Type".to_owned(), "application/json".to_owned()),
        ("Accept".to_owned(), "application/json".to_owned()),
        ("Authorization".to_owned(), format!("Bearer {token}")),
        (
            "X-Snowflake-Authorization-Token-Type".to_owned(),
            auth_type.to_owned(),
        ),
    ]);
    if is_claude_model(model) {
        headers.insert("anthropic-version".to_owned(), "2023-06-01".to_owned());
    }
    Ok(())
}

#[must_use]
pub fn provider_info(
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

mod tests;

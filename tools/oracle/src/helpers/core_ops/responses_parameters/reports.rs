use std::collections::BTreeMap;

use betterlite_context::non_default_params::Params;
use betterlite_core::api::responses_parameters::mcp_headers::extract_mcp_headers;
use betterlite_core::api::responses_parameters::prompt_merge::{
    PromptMergeWarning, ReturnedMessage, input_to_chat_messages, merge_prompt_management_input,
};
use betterlite_core::api::responses_parameters::requested::requested_optional_params;
use betterlite_core::api::responses_parameters::text_format::text_format_to_text;
use betterlite_models::call_helpers::drop_params::{drop_params_env_flag, drop_params_flag};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json as DecisionJson;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use super::helper_error;
use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
pub(crate) struct RequestedInput {
    pub(super) steps: Vec<RequestedStep>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RequestedStep {
    params: Params,
    text_format: Option<JsonValue>,
    text: Option<JsonValue>,
}

pub(super) fn requested_step(fields: &RequestedStep) -> Result<Value, Box<HelperError>> {
    let mut params = fields.params.clone();
    let optional = match requested_optional_params(&mut params) {
        Ok(optional) => optional,
        Err(error) => return Err(helper_error(&error)),
    };
    let converted = text_format_to_text(fields.text_format.as_ref(), fields.text.as_ref());
    let failed = converted.is_err();
    let text = converted.ok().flatten();
    Ok(json!({"optional": optional, "remaining": params, "text": text, "text_failed": failed}))
}

#[derive(Debug, Deserialize)]
pub(crate) struct PromptInput {
    pub(super) steps: Vec<PromptStep>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PromptStep {
    original_input: JsonValue,
    merged: Vec<MergedEntry>,
}

#[derive(Debug, Deserialize)]
struct MergedEntry {
    source: Option<usize>,
    message: JsonValue,
}

pub(super) fn prompt_step(fields: &PromptStep) -> Result<Value, Box<HelperError>> {
    let client = input_to_chat_messages(Some(&fields.original_input))
        .map_err(|error| helper_error(&error))?;
    let messages: Vec<JsonValue> = client.iter().map(|entry| entry.message.clone()).collect();
    if fields.merged.is_empty() {
        return Ok(json!({"client": messages, "input": null, "warnings": Vec::<String>::new()}));
    }
    let merged: Vec<ReturnedMessage> = fields
        .merged
        .iter()
        .map(|entry| ReturnedMessage {
            source: entry.source,
            message: entry.message.clone(),
        })
        .collect();
    let outcome = merge_prompt_management_input(&fields.original_input, &merged)
        .map_err(|error| helper_error(&error))?;
    let warnings: Vec<&str> = outcome
        .warning
        .map(PromptMergeWarning::text)
        .into_iter()
        .collect();
    Ok(json!({"client": messages, "input": outcome.input, "warnings": warnings}))
}

#[derive(Debug, Deserialize)]
pub(crate) struct ForwardedInput {
    pub(super) steps: Vec<ForwardedStep>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ForwardedStep {
    pub(super) extra: Option<BTreeMap<String, JsonValue>>,
    pub(super) client: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct McpInput {
    pub(super) steps: Vec<McpStep>,
}

#[derive(Debug, Deserialize)]
pub(super) struct McpStep {
    secret_fields: Option<JsonValue>,
    tools: Option<Vec<JsonValue>>,
}

pub(super) fn mcp_step(fields: &McpStep) -> Result<Value, Box<HelperError>> {
    let headers = extract_mcp_headers(
        fields.secret_fields.as_ref(),
        fields.tools.as_deref(),
        "x-mcp-auth",
    )
    .map_err(|error| helper_error(&error))?;
    Ok(json!({
        "auth_header": headers.auth_header.as_ref().map(ExposeSecret::expose_secret),
        "server_auth_headers": headers.server_auth_headers.as_ref().map(expose_servers),
        "oauth2_headers": headers.oauth2_headers.as_ref().map(expose_secrets),
        "raw_headers": headers.raw_headers,
    }))
}

fn expose_servers(
    servers: &BTreeMap<String, BTreeMap<String, SecretString>>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    servers
        .iter()
        .map(|(alias, entries)| (alias.clone(), expose_secrets(entries)))
        .collect()
}

fn expose_secrets(values: &BTreeMap<String, SecretString>) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, secret)| (name.clone(), secret.expose_secret().to_owned()))
        .collect()
}

#[derive(Debug, Deserialize)]
pub(crate) struct DropFlagsInput {
    pub(super) steps: Vec<DropFlagsStep>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DropFlagsStep {
    value: Option<DecisionJson>,
    environment: BTreeMap<String, String>,
}

pub(super) fn drop_flags(fields: &DropFlagsStep) -> Value {
    let environment = fields.environment.get("LITELLM_DROP_PARAMS");
    let enabled = match &fields.value {
        None | Some(DecisionJson::Null) => drop_params_env_flag(environment.map(String::as_str)),
        Some(value) => drop_params_flag(Some(value)),
    };
    json!({"drop_params": enabled.enabled})
}

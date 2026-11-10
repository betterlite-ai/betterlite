use betterlite_openai::base_helpers::provider_config;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;

use crate::azure_chat_base_policy::AzureChatSettings;
use crate::azure_chat_base_policy::error::PolicyError;
use crate::azure_chat_base_policy::mapping::{MapPolicy, map_openai_params};
use crate::azure_chat_base_policy::messages::transform_request;

pub(super) const TOOL_UPGRADE: &str = "Azure does not support 'tool_choice', for api_version=2022-01-01. Bump your API version to '2023-12-01-preview' or later. This parameter requires 'api_version=\"2023-12-01-preview\"' or later. Azure API Reference: https://learn.microsoft.com/en-us/azure/ai-services/openai/reference#chat-completions";

pub(super) const REQUIRED_UPGRADE: &str = "Azure does not support 'required' as a tool_choice param, for api_version=2024-05-01. To drop 'tool_choice=required' for calls with this Azure API version, set `litellm.drop_params=True` or for proxy:\n\n`litellm_settings:\n drop_params: true`\nAzure API Reference: https://learn.microsoft.com/en-us/azure/ai-services/openai/reference#chat-completions";

#[derive(Clone, Copy)]
pub(super) enum DropMode {
    Off,
    PerCall,
    Global,
}

pub(super) fn policy<'call>(
    model: &'call str,
    api_version: &'call str,
    mode: DropMode,
) -> MapPolicy<'call> {
    MapPolicy {
        model,
        api_version,
        drop_params: matches!(mode, DropMode::PerCall),
        global_drop_params: matches!(mode, DropMode::Global),
    }
}

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

pub(super) fn parse_messages(text: &str) -> Vec<Message> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn mapped(non_default: &Object, initial: &str, policy: &MapPolicy<'_>) -> Object {
    let mut optional = object(initial);
    map_openai_params(non_default, &mut optional, policy).unwrap();
    optional
}

pub(super) fn blocked(non_default: &Object, initial: &str, policy: &MapPolicy<'_>) -> PolicyError {
    let mut optional = object(initial);
    map_openai_params(non_default, &mut optional, policy).unwrap_err()
}

pub(super) fn transformed(optional: &Object, messages: &[Message], system_first: bool) -> Object {
    transform_request("gpt-4o", messages, optional, system_first).unwrap()
}

pub(super) fn body(optional: &str, messages: &str, system_first: bool) -> Object {
    transformed(&object(optional), &parse_messages(messages), system_first)
}

pub(super) fn tool_choice_error(api_version: &str) -> PolicyError {
    blocked(
        &object(r#"{"tool_choice":"auto"}"#),
        "{}",
        &policy("gpt-4o", api_version, DropMode::Off),
    )
}

pub(super) fn required_tool_choice_error(api_version: &str) -> PolicyError {
    blocked(
        &object(r#"{"tool_choice":"required"}"#),
        "{}",
        &policy("gpt-4o", api_version, DropMode::Off),
    )
}

pub(super) fn promoted(text: &str) -> Object {
    let settings: AzureChatSettings = serde_json::from_str(text).unwrap();
    let projected = provider_config(&settings).unwrap();
    let value = serde_json::to_value(projected).unwrap();
    serde_json::from_value(value).unwrap()
}

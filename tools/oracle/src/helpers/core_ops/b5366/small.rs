use betterlite_core::message_utilities::small_helpers::{
    CustomPromptTemplate, EMPTY_USAGE, PromptTemplateRegistry, get_first_chars_messages,
    register_prompt_template, should_run_mock_completion,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Call {
    RegisterPromptTemplate(TemplateArgs),
    GetFirstCharsMessages(FirstCharsArgs),
    GetEmptyUsage,
    ShouldRunMockCompletion(MockCompletionArgs),
}

#[derive(Debug, Deserialize)]
pub(crate) struct TemplateArgs {
    model: String,
    #[serde(default)]
    resolved_model: Option<String>,
    roles: Json,
    #[serde(default)]
    initial_prompt_value: String,
    #[serde(default)]
    final_prompt_value: String,
    #[serde(default)]
    tokenizer_config: Option<Json>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FirstCharsArgs {
    #[serde(default)]
    messages: Option<Json>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MockCompletionArgs {
    #[serde(default, rename = "mock_response")]
    response: Option<Json>,
    #[serde(default, rename = "mock_tool_calls")]
    tool_calls: Option<Json>,
    #[serde(default, rename = "mock_timeout")]
    timeout: Option<Json>,
}

pub(super) fn run(call: &Call) -> serde_json::Value {
    match call {
        Call::RegisterPromptTemplate(args) => template(args),
        Call::GetFirstCharsMessages(args) => {
            let mut kwargs = IndexMap::new();
            kwargs.insert(
                String::from("messages"),
                args.messages.clone().unwrap_or(Json::Null),
            );
            json!(get_first_chars_messages(&Json::Object(kwargs)))
        }
        Call::GetEmptyUsage => json!(EMPTY_USAGE),
        Call::ShouldRunMockCompletion(args) => json!(should_run_mock_completion(
            &args.response.clone().unwrap_or(Json::Null),
            &args.tool_calls.clone().unwrap_or(Json::Null),
            &args.timeout.clone().unwrap_or(Json::Null),
        )),
    }
}

fn template(args: &TemplateArgs) -> serde_json::Value {
    let mut registry = PromptTemplateRegistry::default();
    let template = CustomPromptTemplate {
        roles: args.roles.clone(),
        initial_prompt_value: args.initial_prompt_value.clone(),
        final_prompt_value: args.final_prompt_value.clone(),
    };
    let custom = register_prompt_template(
        &mut registry,
        &args.model,
        args.resolved_model.as_deref(),
        &template,
        args.tokenizer_config.as_ref(),
    );
    json!({
        "custom": custom,
        "known": registry.known_tokenizer_config.clone(),
    })
}

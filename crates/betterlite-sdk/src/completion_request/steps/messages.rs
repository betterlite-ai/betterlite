use super::convert::{builtin_messages, convert, messages_from_builtin, typed_from};
use crate::completion_options::tool_keys::{drop_anthropic_only_tool_keys, is_claude_tool_target};
use crate::completion_request::Preparation;
use crate::completion_request::args::JsonObject;
use crate::completion_request::error::{CompletionRequestError, RequestResult};
use betterlite_core::message_utilities::validators::{
    StopSequences, fix_openai_messages, fix_thinking_param, validate_and_fix_openai_tools,
    validate_optional_params, validate_tool_choice,
};
use betterlite_models::decisions::Json;
use betterlite_prompts::builtin::system::map_system_message_pt;
use betterlite_prompts::file_ids::mapping::FileIdMapping;
use betterlite_prompts::file_ids::messages::update_messages_with_file_ids;
use betterlite_prompts::message_content::Message;
use betterlite_prompts::message_fields::system_prompt::add_system_prompt_to_messages;
use betterlite_prompts::messages::get_completion_messages;
use serde::de::DeserializeOwned;
pub(crate) const fn text_of(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
pub(crate) fn optional<T>(kwargs: &JsonObject, name: &'static str) -> RequestResult<Option<T>>
where
    T: DeserializeOwned,
{
    match kwargs.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => Ok(Some(typed_from::<T>(value, name)?)),
    }
}
pub(crate) fn take_flag(kwargs: &mut JsonObject, name: &'static str) -> RequestResult<bool> {
    match kwargs.shift_remove(name) {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(flag)) => Ok(flag),
        Some(_) => Err(CompletionRequestError::FieldType {
            field: name,
            expected: "a boolean",
        }),
    }
}
pub(crate) fn validate(state: &mut Preparation<'_>) -> RequestResult<()> {
    let Some(model) = state.args.model.clone() else {
        return Err(CompletionRequestError::ModelNotPassed);
    };
    let validated = fix_openai_messages(&state.args.messages)?;
    let tools = validate_and_fix_openai_tools(state.args.tools.as_deref());
    let tool_choice = state
        .args
        .tool_choice
        .as_ref()
        .map(validate_tool_choice)
        .transpose()?;
    let stop = match &state.args.stop {
        Some(value) => validate_optional_params(
            Some(typed_from::<StopSequences>(value, "stop")?),
            state.settings.flags.disable_stop_sequence_limit,
        ),
        None => None,
    };
    let thinking = state
        .args
        .thinking
        .as_ref()
        .and_then(|value| fix_thinking_param(value, state.settings.medium_thinking_budget));
    state.call.model = model;
    state.call.messages = convert(&validated)?;
    state.call.tools.clone_from(&tools);
    let mut snapshot = state.args.clone();
    snapshot.messages = validated;
    snapshot.tools = tools;
    snapshot.tool_choice.clone_from(&tool_choice);
    snapshot.stop = stop.as_ref().map(convert).transpose()?;
    snapshot.thinking.clone_from(&thinking);
    state.snapshot = snapshot;
    state.tool_choice = tool_choice;
    state.stop = stop;
    state.thinking = thinking;
    Ok(())
}
fn continue_messages(kwargs: &JsonObject) -> RequestResult<(Option<Message>, Option<Message>)> {
    let user = optional::<Message>(kwargs, "user_continue_message")?;
    let assistant = optional::<Message>(kwargs, "assistant_continue_message")?;
    Ok((user, assistant))
}
fn alternated(state: &Preparation<'_>) -> RequestResult<Vec<Message>> {
    let (user, assistant) = continue_messages(&state.kwargs)?;
    let ensure = optional::<bool>(&state.kwargs, "ensure_alternating_roles")?.unwrap_or(false);
    let messages = &state.call.messages;
    get_completion_messages(messages, assistant.as_ref(), user.as_ref(), ensure)
        .map_err(CompletionRequestError::from)
}
pub(crate) fn alternation(state: &mut Preparation<'_>) -> RequestResult<()> {
    state.call.messages = alternated(state)?;
    Ok(())
}
pub(crate) fn system_prompt(state: &mut Preparation<'_>) -> RequestResult<()> {
    let Some(prompt) = optional::<String>(&state.kwargs, "litellm_system_prompt")? else {
        return Ok(());
    };
    state.call.messages = add_system_prompt_to_messages(&state.call.messages, &prompt, true);
    Ok(())
}
pub(crate) fn file_ids(state: &mut Preparation<'_>) -> RequestResult<()> {
    let model_info = optional::<JsonObject>(&state.kwargs, "model_info")?;
    let model_id = model_info
        .as_ref()
        .and_then(|info| info.get("id"))
        .and_then(text_of);
    let mapping =
        optional::<FileIdMapping>(&state.kwargs, "model_file_id_mapping")?.unwrap_or_default();
    update_messages_with_file_ids(&mut state.call.messages, model_id, &mapping)?;
    Ok(())
}
pub(crate) fn developer_role(state: &mut Preparation<'_>, provider: &str) {
    let policy = state.ports.model_metadata.chat_policy(
        &state.call.model,
        provider,
        state.call.base_model.as_deref(),
    );
    if !policy.translates_developer_role {
        return;
    }
    for message in &mut state.call.messages {
        if message.role == "developer" {
            message.role = String::from("system");
            message.name = None;
            message.provider_specific_fields.clear();
        }
    }
}
pub(crate) fn system_messages(state: &mut Preparation<'_>) -> RequestResult<()> {
    let Some(false) = optional::<bool>(&state.kwargs, "supports_system_message")? else {
        return Ok(());
    };
    let messages = builtin_messages(&state.call.messages)?;
    state.call.messages = messages_from_builtin(&map_system_message_pt(&messages)?)?;
    Ok(())
}
pub(crate) fn drop_tool_keys(state: &mut Preparation<'_>, provider: &str) {
    if is_claude_tool_target(Some(provider), &state.call.model) {
        return;
    }
    state.call.tools = drop_anthropic_only_tool_keys(state.call.tools.as_deref());
}

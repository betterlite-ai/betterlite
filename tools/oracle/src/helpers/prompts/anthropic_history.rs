use betterlite_prompts::anthropic_history;
use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    SanitizeEmptyTextContent {
        message: Json,
    },
    AddMissingToolResults {
        current_message: Json,
        messages: Vec<Json>,
        current_index: usize,
    },
    IsOrphanedToolResult {
        current_message: Json,
        sanitized_messages: Vec<Json>,
    },
    DeclaredToolCallIds {
        message: Json,
    },
    GroupToolExchanges {
        messages: Vec<Json>,
    },
    IterToolExchangeGroups {
        messages: Vec<Json>,
    },
    SanitizeMessagesForToolCalling {
        messages: Vec<Json>,
        modify_params: bool,
    },
    DropUnsignableThinkingBlocks {
        thinking_blocks: Vec<Json>,
    },
    RepairIndices {
        messages: Vec<Json>,
        modify_params: bool,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::SanitizeEmptyTextContent { message } => ok_value(
                anthropic_history::text::sanitize_empty_text_content(message),
            ),
            Input::AddMissingToolResults {
                current_message,
                messages,
                current_index,
            } => add_missing_tool_results(current_message, messages, *current_index),
            Input::IsOrphanedToolResult {
                current_message,
                sanitized_messages,
            } => orphaned_tool_result(current_message, sanitized_messages),
            Input::DeclaredToolCallIds { message } => declared_ids(message),
            Input::GroupToolExchanges { messages } => {
                ok_value(anthropic_history::groups::group_tool_exchanges(messages))
            }
            Input::IterToolExchangeGroups { messages } => ok_value(
                anthropic_history::groups::iter_tool_exchange_groups(messages).collect::<Vec<_>>(),
            ),
            Input::SanitizeMessagesForToolCalling {
                messages,
                modify_params,
            } => sanitize_messages(messages, *modify_params),
            Input::DropUnsignableThinkingBlocks { thinking_blocks } => ok_value(
                anthropic_history::thinking::drop_unsignable(thinking_blocks),
            ),
            Input::RepairIndices {
                messages,
                modify_params,
            } => ok_value(anthropic_history::groups::repair_indices(
                messages,
                *modify_params,
            )),
        }
    }
}

fn add_missing_tool_results(
    current_message: &Json,
    messages: &[Json],
    current_index: usize,
) -> crate::helpers::HelperResult {
    ok_value(anthropic_history::tools::add_missing_tool_results(
        current_message,
        messages,
        current_index,
    ))
}

fn orphaned_tool_result(
    current_message: &Json,
    sanitized_messages: &[Json],
) -> crate::helpers::HelperResult {
    ok_value(anthropic_history::tools::is_orphaned_tool_result(
        current_message,
        sanitized_messages,
    ))
}

fn declared_ids(message: &Json) -> crate::helpers::HelperResult {
    let mut ids = anthropic_history::groups::declared_tool_call_ids(message);
    ids.sort();
    ok_value(ids)
}

fn sanitize_messages(messages: &[Json], modify_params: bool) -> crate::helpers::HelperResult {
    let sanitized = anthropic_history::tools::sanitize_messages(messages, modify_params).map_err(
        |failure| {
            Box::new(crate::helpers::HelperError {
                class: "AnthropicHistory".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: failure.to_string(),
            })
        },
    )?;
    ok_value(sanitized)
}

fn ok_value<Payload>(payload: Payload) -> crate::helpers::HelperResult
where
    Payload: Serialize,
{
    serde_json::to_value(payload).map_err(|failure| {
        Box::new(crate::helpers::HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    })
}

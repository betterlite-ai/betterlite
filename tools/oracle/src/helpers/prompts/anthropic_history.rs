use betterlite_prompts::anthropic_history;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    SanitizeEmptyTextContent {
        message: Value,
    },
    AddMissingToolResults {
        current_message: Value,
        messages: Vec<Value>,
        current_index: usize,
    },
    IsOrphanedToolResult {
        current_message: Value,
        sanitized_messages: Vec<Value>,
    },
    DeclaredToolCallIds {
        message: Value,
    },
    GroupToolExchanges {
        messages: Vec<Value>,
    },
    IterToolExchangeGroups {
        messages: Vec<Value>,
    },
    SanitizeMessagesForToolCalling {
        messages: Vec<Value>,
        modify_params: bool,
    },
    DropUnsignableThinkingBlocks {
        thinking_blocks: Vec<Value>,
    },
    RepairIndices {
        messages: Vec<Value>,
        modify_params: bool,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input {
            Input::SanitizeEmptyTextContent { message } => {
                anthropic_history::text::sanitize_empty_text_content(message)
            }
            Input::AddMissingToolResults {
                current_message,
                messages,
                current_index,
            } => {
                json!(anthropic_history::tools::add_missing_tool_results(
                    current_message,
                    messages,
                    *current_index
                ))
            }
            Input::IsOrphanedToolResult {
                current_message,
                sanitized_messages,
            } => Value::Bool(anthropic_history::tools::is_orphaned_tool_result(
                current_message,
                sanitized_messages,
            )),
            Input::DeclaredToolCallIds { message } => {
                let mut ids = anthropic_history::groups::declared_tool_call_ids(message);
                ids.sort();
                Value::Array(ids.into_iter().map(Value::String).collect())
            }
            Input::GroupToolExchanges { messages } => {
                json!(anthropic_history::groups::group_tool_exchanges(messages))
            }
            Input::IterToolExchangeGroups { messages } => json!(
                anthropic_history::groups::iter_tool_exchange_groups(messages).collect::<Vec<_>>()
            ),
            Input::SanitizeMessagesForToolCalling {
                messages,
                modify_params,
            } => json!(
                anthropic_history::tools::sanitize_messages(messages, *modify_params).map_err(
                    |failure| {
                        Box::new(crate::helpers::HelperError {
                            class: "AnthropicHistory".to_owned(),
                            status: None,
                            error_type: None,
                            param: None,
                            code: None,
                            message: failure.to_string(),
                        })
                    }
                )?
            ),
            Input::DropUnsignableThinkingBlocks { thinking_blocks } => {
                json!(anthropic_history::thinking::drop_unsignable(
                    thinking_blocks
                ))
            }
            Input::RepairIndices {
                messages,
                modify_params,
            } => json!(anthropic_history::groups::repair_indices(
                messages,
                *modify_params
            )),
        };
        Ok(result)
    }
}

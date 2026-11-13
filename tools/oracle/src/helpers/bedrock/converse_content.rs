use betterlite_bedrock::converse_content::citations::citations_to_annotations;
use betterlite_bedrock::converse_content::json_mode::{
    filter_json_mode_tools, unwrap_bedrock_properties,
};
use betterlite_bedrock::converse_content::recovery::{
    AssistantMessage, ToolCallRecovery, recover_tool_call,
};
use betterlite_bedrock::converse_content::translation::{Translation, translate_message_content};
use betterlite_bedrock::converse_content::{
    ContentBlock, ToolBlock, ToolCall, get_tool_call_names,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::helpers::{HelperError, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    ReverseToolNames {
        #[serde(default)]
        tools: Option<Vec<ToolBlock>>,
    },
    JsonModeFilter {
        #[serde(default)]
        json_mode: Option<bool>,
        #[serde(default)]
        tools: Option<Vec<ToolCall>>,
        #[serde(default)]
        message: AssistantMessage,
    },
    WrappedProperties {
        value: String,
    },
    TextWithToolUse {
        content_blocks: Vec<ContentBlock>,
        #[serde(default)]
        mappings: IndexMap<String, String>,
    },
    EmptyToolInput {
        content_blocks: Vec<ContentBlock>,
        #[serde(default)]
        mappings: IndexMap<String, String>,
    },
    ContentFunctionRecovery {
        #[serde(default)]
        message: AssistantMessage,
        #[serde(default)]
        tools: Option<Vec<ToolBlock>>,
        #[serde(default)]
        initial_finish_reason: Option<String>,
    },
    CitationSearchLocations {
        #[serde(default)]
        citations_content_blocks: Option<Vec<Json>>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Operation;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Operation::ReverseToolNames { tools } => reverse_tool_names(tools.as_deref()),
            Operation::JsonModeFilter {
                json_mode,
                tools,
                message,
            } => json_mode_filter(*json_mode, tools.as_deref(), message),
            Operation::WrappedProperties { value } => payload(unwrap_bedrock_properties(value)),
            Operation::TextWithToolUse {
                content_blocks,
                mappings,
            } => payload(translation_result(content_blocks, mappings)?),
            Operation::EmptyToolInput {
                content_blocks,
                mappings,
            } => {
                let tools = translation_result(content_blocks, mappings)?.tools;
                payload(json!({ "tools": tools }))
            }
            Operation::ContentFunctionRecovery {
                message,
                tools,
                initial_finish_reason,
            } => content_function_recovery(
                message,
                tools.as_deref(),
                initial_finish_reason.as_deref(),
            ),
            Operation::CitationSearchLocations {
                citations_content_blocks,
            } => payload(citations_to_annotations(
                citations_content_blocks.as_deref(),
            )),
        }
    }
}

fn reverse_tool_names(tools: Option<&[ToolBlock]>) -> HelperResult {
    let mut names = get_tool_call_names(tools);
    names.sort();
    payload(names)
}

fn json_mode_filter(
    json_mode: Option<bool>,
    tools: Option<&[ToolCall]>,
    message: &AssistantMessage,
) -> HelperResult {
    payload(filter_json_mode_tools(
        json_mode.unwrap_or(false),
        tools.unwrap_or_default(),
        message.content.as_deref(),
    ))
}

fn translation_result(
    content_blocks: &[ContentBlock],
    mappings: &IndexMap<String, String>,
) -> Result<Translation, Box<HelperError>> {
    match translate_message_content(content_blocks, mappings) {
        Ok(translation) => Ok(translation),
        Err(error) => Err(helper_failure("ConverseContentError", error.to_string())),
    }
}

fn content_function_recovery(
    message: &AssistantMessage,
    tools: Option<&[ToolBlock]>,
    initial_finish_reason: Option<&str>,
) -> HelperResult {
    let mut recovered = message.clone();
    let finish_reason = recover_tool_call(&mut recovered, tools, initial_finish_reason, &Uuid::nil);
    payload(ToolCallRecovery {
        message: recovered,
        finish_reason,
    })
}

fn payload<Payload>(value: Payload) -> HelperResult
where
    Payload: serde::Serialize,
{
    match serde_json::to_value(value) {
        Ok(json) => Ok(json),
        Err(error) => Err(helper_failure("SerializationError", error.to_string())),
    }
}

fn helper_failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

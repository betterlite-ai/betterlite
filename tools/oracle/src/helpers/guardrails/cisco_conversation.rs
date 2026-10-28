use betterlite_guardrails::cisco_conversation::json::Json;
use betterlite_guardrails::cisco_conversation::request::{
    flatten_input, request_messages, tool_argument_parts, tool_definition_text,
};
use betterlite_guardrails::cisco_conversation::response::{
    chat_messages, completion_messages, output_messages, reasoning_parts,
};
use betterlite_guardrails::cisco_conversation::rewrite::{messages_to_input, rewrite_input_text};
use betterlite_guardrails::cisco_conversation::{Fields, normalize_content};
use serde_json::value::RawValue;
use serde_json::{Map, Value, json};

use crate::helpers::HelperError;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    NormalizeContent {
        content: Value,
    },
    ToolArguments {
        message: Map<String, Value>,
    },
    Reasoning {
        message: Map<String, Value>,
    },
    Flatten {
        input: Value,
    },
    RequestMessages {
        data: Map<String, Value>,
    },
    ToolDefinitions {
        data: Map<String, Value>,
    },
    ChatResponse {
        response: Map<String, Value>,
    },
    TextCompletion {
        response: Map<String, Value>,
    },
    ResponsesOutput {
        response: Map<String, Value>,
    },
    ToResponses {
        messages: Vec<Value>,
    },
    Rewrite {
        input: Value,
        sanitized_text: String,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        outcome(input.clone()).map_err(|error| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn outcome(call: Input) -> Result<Value, serde_json::Error> {
    let fields = |map: Map<String, Value>| serde_json::from_value::<Fields>(Value::Object(map));
    let parsed = |value: Value| {
        serde_json::from_value::<Box<RawValue>>(value).map(|raw| Json::parse(raw.get()))
    };
    Ok(match call {
        Input::NormalizeContent { content } => json!(normalize_content(&parsed(content)?)),
        Input::ToolArguments { message } => json!(tool_argument_parts(&fields(message)?)),
        Input::Reasoning { message } => json!(reasoning_parts(&fields(message)?)),
        Input::Flatten { input } => json!(flatten_input(&parsed(input)?)),
        Input::RequestMessages { data } => json!(request_messages(&fields(data)?)),
        Input::ToolDefinitions { data } => json!(tool_definition_text(&fields(data)?)),
        Input::ChatResponse { response } => json!(chat_messages(&fields(response)?)),
        Input::TextCompletion { response } => json!(completion_messages(&fields(response)?)),
        Input::ResponsesOutput { response } => json!(output_messages(&fields(response)?)),
        Input::ToResponses { messages } => {
            let items = serde_json::from_value::<Vec<Box<RawValue>>>(Value::Array(messages))?;
            json!(messages_to_input(&items))
        }
        Input::Rewrite {
            input,
            sanitized_text,
        } => {
            json!(rewrite_input_text(&parsed(input)?, &sanitized_text))
        }
    })
}

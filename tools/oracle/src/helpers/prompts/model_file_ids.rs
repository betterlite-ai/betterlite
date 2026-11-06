use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_prompts::file_ids::managed_id::get_format_from_file_id;
use betterlite_prompts::file_ids::mapping::FileIdMapping;
use betterlite_prompts::file_ids::messages::{
    get_file_ids_from_messages, update_messages_with_file_ids,
};
use betterlite_prompts::file_ids::responses::{ResponsesInput, update_responses_file_ids};
use betterlite_prompts::file_ids::tools::{
    ResponsesTool, decode_vector_store_ids, update_tools_with_file_ids,
};
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde_json::to_value;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "get_format_from_file_id")]
    Format {
        #[serde(default)]
        file_id: Option<String>,
    },
    #[serde(rename = "update_messages_with_model_file_ids")]
    Messages {
        messages: Vec<Message>,
        #[serde(default)]
        model_id: Option<String>,
        #[serde(default)]
        mapping: FileIdMapping,
    },
    #[serde(rename = "update_responses_input_with_model_file_ids")]
    Responses {
        #[serde(rename = "input")]
        request: ResponsesInput,
        #[serde(default)]
        model_id: Option<String>,
        #[serde(default)]
        mapping: FileIdMapping,
    },
    #[serde(rename = "decode_vector_store_ids_in_tools")]
    DecodeTools {
        #[serde(default)]
        tools: Option<Vec<ResponsesTool>>,
    },
    #[serde(rename = "update_responses_tools_with_model_file_ids")]
    Tools {
        #[serde(default)]
        tools: Option<Vec<ResponsesTool>>,
        #[serde(default)]
        model_id: Option<String>,
        #[serde(default)]
        mapping: FileIdMapping,
    },
    #[serde(rename = "get_file_ids_from_messages")]
    FileIds { messages: Vec<Message> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Format { file_id } => ok_value(get_format_from_file_id(file_id.as_deref())),
            Input::Messages {
                messages,
                model_id,
                mapping,
            } => {
                let mut updated = messages.clone();
                update_messages_with_file_ids(&mut updated, model_id.as_deref(), mapping);
                ok_value(updated)
            }
            Input::Responses {
                request,
                model_id,
                mapping,
            } => ok_value(update_responses_file_ids(
                request,
                model_id.as_deref(),
                mapping,
            )),
            Input::DecodeTools { tools } => ok_value(decode_vector_store_ids(tools.as_deref())),
            Input::Tools {
                tools,
                model_id,
                mapping,
            } => ok_value(update_tools_with_file_ids(
                tools.as_deref(),
                model_id.as_deref(),
                mapping,
            )),
            Input::FileIds { messages } => ok_value(get_file_ids_from_messages(messages)),
        }
    }
}

fn ok_value<Payload>(payload: Payload) -> HelperResult
where
    Payload: serde::Serialize,
{
    match to_value(payload) {
        Ok(json) => Ok(json),
        Err(error) => Err(Box::new(HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })),
    }
}

mod tests;

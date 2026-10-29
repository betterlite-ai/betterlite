use betterlite_core::api::responses_input_content::{
    ContentError, chat_content_type, content_blocks, input_file_block, input_image_block,
    instructions_to_system_message, resolve_file_id,
};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum Input {
    ResolveFileId {
        item: Map<String, Value>,
    },
    FileBlock {
        item: Map<String, Value>,
    },
    ImageBlock {
        item: Map<String, Value>,
    },
    ContentBlocks {
        content: Value,
    },
    ContentTypes {
        content_types: Vec<String>,
        instructions: Vec<Option<String>>,
    },
    InstructionsToSystemMessage {
        instructions: Option<String>,
    },
}

pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ResolveFileId { item } => Ok(resolve_file_id(item).unwrap_or(Value::Null)),
            Input::FileBlock { item } => Ok(input_file_block(item)),
            Input::ImageBlock { item } => Ok(input_image_block(item)),
            Input::ContentBlocks { content } => {
                content_blocks(content).map_err(|error| invalid_content_error(&error))
            }
            Input::ContentTypes {
                content_types,
                instructions,
            } => Ok(content_types_result(content_types, instructions)),
            Input::InstructionsToSystemMessage { instructions } => {
                Ok(instructions_to_system_message(instructions.as_deref()))
            }
        }
    }
}

fn content_types_result(content_types: &[String], instructions: &[Option<String>]) -> Value {
    let mapped_types = content_types
        .iter()
        .map(|content_type| Value::String(chat_content_type(content_type).to_owned()))
        .collect();
    let mapped_instructions = instructions
        .iter()
        .map(|instruction| instructions_to_system_message(instruction.as_deref()))
        .collect();
    Value::Object(Map::from_iter([
        ("content_types".to_owned(), Value::Array(mapped_types)),
        ("instructions".to_owned(), Value::Array(mapped_instructions)),
    ]))
}

fn invalid_content_error(error: &ContentError) -> Box<HelperError> {
    let message = error.to_string();
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

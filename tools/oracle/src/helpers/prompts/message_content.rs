use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_openai::chat::content::from_model_response;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::{
    Message, any_messages::to_str_messages, convert_content_list_to_str,
    extract_search_results_text, image::request_contains_image_content,
    projection::to_content_only_messages, prompt::to_messages,
};
use serde::Deserialize;
use serde_json::to_value;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    operation: String,
    #[serde(default)]
    message: Option<Message>,
    #[serde(default)]
    messages: Vec<Message>,
    #[serde(default)]
    search_results: Option<Json>,
    #[serde(default = "default_json_null")]
    prompt: Json,
    #[serde(default = "default_json_null")]
    any_messages: Json,
    #[serde(default)]
    response: Option<ChatResponse>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let result = match input.operation.as_str() {
            "convert_content_list_to_str" => to_value(convert_content_list_to_str(required_field(
                input.message.as_ref(),
                "message",
            )?)),
            "extract_search_results_text" => {
                to_value(extract_search_results_text(input.search_results.as_ref()))
            }
            "request_contains_image_content" => {
                to_value(request_contains_image_content(&input.messages))
            }
            "convert_openai_message_to_only_content_messages" => {
                to_value(to_content_only_messages(&input.messages))
            }
            "text_completion_prompt_to_messages" => {
                let messages = to_messages(&input.prompt)
                    .map_err(|error| classified_error("ValueError", error.to_string()))?;
                to_value(messages)
            }
            "handle_any_messages_to_chat_completion_str_messages_conversion" => {
                to_value(to_str_messages(&input.any_messages))
            }
            "get_content_from_model_response" => to_value(from_model_response(required_field(
                input.response.as_ref(),
                "response",
            )?)),
            operation => {
                return Err(classified_error(
                    "InvalidInput",
                    format!("unsupported operation {operation}"),
                ));
            }
        };
        result.map_err(|error| classified_error("SerializationError", error.to_string()))
    }
}

fn required_field<'input, T>(
    field: Option<&'input T>,
    name: &str,
) -> Result<&'input T, Box<HelperError>> {
    let Some(value) = field else {
        return Err(classified_error(
            "InvalidInput",
            format!("{name} is required"),
        ));
    };
    Ok(value)
}

const fn default_json_null() -> Json {
    Json::Null
}

fn classified_error(class: &str, message: String) -> Box<HelperError> {
    let error_class = class.to_owned();
    Box::new(HelperError {
        class: error_class,
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

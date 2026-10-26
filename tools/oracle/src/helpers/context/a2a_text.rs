use betterlite_context::a2a_text::{
    A2aResponse, A2aTextError, RecordValue, extract_text_from_a2a_message,
    extract_text_from_a2a_response,
};
use serde_json::json;

use crate::helpers::HelperError;

mod tests;

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Message {
        message: Option<RecordValue>,
        depth: Option<usize>,
        max_depth: Option<usize>,
    },
    Response {
        response: A2aResponse,
        max_depth: Option<usize>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input {
            Input::Message {
                message,
                depth,
                max_depth,
            } => extract_text_from_a2a_message(
                message.as_ref(),
                depth.unwrap_or(0),
                max_depth.unwrap_or(10),
            ),
            Input::Response {
                response,
                max_depth,
            } => extract_text_from_a2a_response(response, max_depth.unwrap_or(10)),
        };
        result.map(|text| json!(text)).map_err(|error| {
            let class = match error {
                A2aTextError::MessageNotObject | A2aTextError::PartNotObject => "AttributeError",
                A2aTextError::PartsNotIterable
                | A2aTextError::TextNotString
                | A2aTextError::ArtifactsNotSized => "TypeError",
                A2aTextError::ArtifactsNotIndexed => "KeyError",
            };
            Box::new(HelperError {
                class: class.to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

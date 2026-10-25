use betterlite_context::a2a_text::{
    A2aObject, A2aValue, extract_text_from_a2a_message, extract_text_from_a2a_response,
};
use serde_json::json;

use crate::helpers::HelperError;

mod tests;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Message {
        message: Option<A2aValue>,
        depth: Option<usize>,
        max_depth: Option<usize>,
    },
    Response {
        response: A2aObject,
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
        result
            .map(|text| json!({"result": text, "logs": []}))
            .map_err(|error| {
                Box::new(HelperError {
                    class: "TypeError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })
    }
}

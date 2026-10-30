use betterlite_core::api::responses_reasoning_merge::{merge_reasoning, merge_trailing};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
struct Input {
    mode: String,
    messages: Vec<serde_json::Value>,
    #[serde(default)]
    new_messages: Vec<serde_json::Value>,
    #[serde(default)]
    hosted_search: bool,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input.mode.as_str() {
            "reasoning" => merge_reasoning(&input.messages),
            "trailing" => merge_trailing(&input.messages, &input.new_messages, input.hosted_search)
                .map_err(|error| {
                    Box::new(crate::helpers::HelperError {
                        class: "InvalidInput".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: error.to_string(),
                    })
                })?
                .map_or(serde_json::Value::Null, |message| message),
            _ => {
                return Err(Box::new(crate::helpers::HelperError {
                    class: "InvalidInput".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: "unknown merge mode".to_owned(),
                }));
            }
        };
        serde_json::to_value(result).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "Encode".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

use betterlite_core::api::responses_reasoning_merge::{merge_reasoning, merge_trailing};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
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
        match input.mode.as_str() {
            "reasoning" => Ok(serde_json::Value::Array(merge_reasoning(&input.messages))),
            "trailing" => merge_trailing(&input.messages, &input.new_messages, input.hosted_search)
                .map(|merged| {
                    // a bare null here would collapse to a missing "value" key once the wire format round-trips through Option<Value>
                    let mut object = serde_json::Map::new();
                    object.insert(
                        "merged".to_owned(),
                        merged.unwrap_or(serde_json::Value::Null),
                    );
                    serde_json::Value::Object(object)
                })
                .map_err(|error| {
                    Box::new(crate::helpers::HelperError {
                        class: "InvalidInput".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: error.to_string(),
                    })
                }),
            _ => Err(Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "unknown merge mode".to_owned(),
            })),
        }
    }
}

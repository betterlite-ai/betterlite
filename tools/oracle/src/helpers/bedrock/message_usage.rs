use betterlite_bedrock::message_usage::{Json, promote_message_usage};
use serde::Deserialize;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    events: Vec<Json>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        serde_json::to_value(promote_message_usage(input.events.clone())).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

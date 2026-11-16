use betterlite_bedrock::message_usage::{StreamItem, promote_message_usage};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    events: Vec<StreamItem>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let promoted = promote_message_usage(input.events.clone())
            .map_err(|error| error.to_string())
            .and_then(|items| serde_json::to_value(items).map_err(|error| error.to_string()));
        promoted.map_err(|message| {
            Box::new(HelperError {
                class: "MessageUsageError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            })
        })
    }
}

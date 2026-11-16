use betterlite_anthropic::web_replay::flatten_web_search_results;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    messages: Vec<Json>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let flattened = flatten_web_search_results(&input.messages);
        serde_json::to_value(flattened).map_err(|error| {
            Box::new(HelperError {
                class: "AnthropicWebReplay".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

mod tests;

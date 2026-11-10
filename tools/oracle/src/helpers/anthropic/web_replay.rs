use betterlite_anthropic::web_replay::flatten_web_search_results;
use betterlite_models::decisions::Json;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    messages: Vec<Json>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let flattened = flatten_web_search_results(&input.messages);
        serde_json::to_value(flattened).map_err(|error| {
            Box::new(crate::helpers::HelperError {
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

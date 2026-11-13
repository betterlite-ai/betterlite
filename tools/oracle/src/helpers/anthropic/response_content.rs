use betterlite_anthropic::response_content::{project_response, wire::AnthropicCompletionResponse};

#[derive(Debug, serde::Deserialize)]
pub(crate) struct Input {
    response: AnthropicCompletionResponse,
    json_mode: Option<bool>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let reply = project_response(&input.response, input.json_mode).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "AnthropicResponseContent".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        serde_json::to_value(reply).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "AnthropicResponseContent".to_owned(),
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

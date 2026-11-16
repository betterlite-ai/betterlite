use betterlite_auth::inference_routes;
use betterlite_auth::inference_routes::{ANTHROPIC_ROUTES, OPENAI_ROUTES};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Group {
    #[serde(rename = "openai_routes")]
    Openai,
    #[serde(rename = "anthropic_routes")]
    Anthropic,
    #[serde(rename = "llm_api_routes")]
    LlmApi,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    group: Group,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let routes = match input.group {
            Group::Openai => OPENAI_ROUTES.to_vec(),
            Group::Anthropic => ANTHROPIC_ROUTES.to_vec(),
            Group::LlmApi => inference_routes::llm_api_routes(),
        };
        serde_json::to_value(routes).map_err(|source| {
            Box::new(HelperError {
                class: "SerializeError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
    }
}

mod tests;

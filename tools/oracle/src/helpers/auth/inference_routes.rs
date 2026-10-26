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

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let routes = match input.group {
            Group::Openai => betterlite_auth::inference_routes::OPENAI_ROUTES.to_vec(),
            Group::Anthropic => betterlite_auth::inference_routes::ANTHROPIC_ROUTES.to_vec(),
            Group::LlmApi => betterlite_auth::inference_routes::llm_api_routes(),
        };
        serde_json::to_value(routes).map_err(|source| {
            Box::new(crate::helpers::HelperError {
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

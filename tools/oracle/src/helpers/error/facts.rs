use betterlite_error::facts::{ExceptionCheckers, ProviderErrorBody, ProviderErrorFacts};
use betterlite_error::parse::{get_body_error_code, get_error_message};
use betterlite_error::status::map_exception_by_status;
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    RateLimit {
        text: String,
        status: Option<u16>,
    },
    ContextWindow {
        text: String,
    },
    BodyCode {
        text: String,
    },
    Message {
        body: Option<ProviderErrorBody>,
    },
    Status {
        status: Option<u16>,
        #[serde(default)]
        status_code_is_synthesized: bool,
        text: String,
        provider: Option<String>,
    },
}

fn message(body: Option<&ProviderErrorBody>) -> Option<String> {
    let facts = ProviderErrorFacts {
        body: body.cloned(),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    get_error_message(&facts)
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::RateLimit { text, status } => {
                serde_json::json!(ExceptionCheckers::is_error_str_rate_limit(text, *status))
            }
            Input::ContextWindow { text } => {
                serde_json::json!(ExceptionCheckers::context_window(text))
            }
            Input::BodyCode { text } => serde_json::json!(get_body_error_code(text)),
            Input::Message { body } => serde_json::json!(message(body.as_ref())),
            Input::Status {
                status,
                status_code_is_synthesized,
                text,
                provider,
            } => {
                let facts = ProviderErrorFacts {
                    status_code: *status,
                    status_code_is_synthesized: *status_code_is_synthesized,
                    body: None,
                    message: None,
                    headers: None,
                    response: None,
                    litellm_response_headers: None,
                    request: None,
                };
                serde_json::json!(map_exception_by_status(
                    &facts,
                    text,
                    provider.as_deref().unwrap_or("Provider")
                ))
            }
        };
        Ok(serde_json::json!({"result": output}))
    }
}

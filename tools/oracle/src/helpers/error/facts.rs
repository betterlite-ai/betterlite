use betterlite_error::facts::{ExceptionCheckers, ProviderErrorBody, ProviderErrorFacts};
use betterlite_error::parse::{get_body_error_code, get_error_message};
use betterlite_error::status::map_exception_by_status;

pub(crate) struct Op;

fn invalid_input(message: String) -> Box<crate::helpers::HelperError> {
    let class = "InvalidInput".to_owned();
    Box::new(crate::helpers::HelperError {
        class,
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn message(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let body_value = input
        .get("body")
        .cloned()
        .ok_or_else(|| invalid_input("message body is missing".to_owned()))?;
    let body: ProviderErrorBody =
        serde_json::from_value(body_value).map_err(|error| invalid_input(error.to_string()))?;
    let facts = ProviderErrorFacts {
        body: Some(body),
        ..ProviderErrorFacts::empty()
    };
    Ok(serde_json::json!(get_error_message(&facts)))
}

fn status(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let status_code = input
        .get("status")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u16::try_from(value).ok());
    let facts = ProviderErrorFacts {
        status_code,
        status_code_is_synthesized: input
            .get("status_code_is_synthesized")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        ..ProviderErrorFacts::empty()
    };
    serde_json::to_value(map_exception_by_status(
        &facts,
        input
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        input
            .get("provider")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Provider"),
    ))
    .map_err(|error| invalid_input(error.to_string()))
}

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let kind = input
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let text = input
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let output = match kind {
            "rate_limit" => serde_json::json!(ExceptionCheckers::is_error_str_rate_limit(
                text,
                input
                    .get("status")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|value| u16::try_from(value).ok()),
            )),
            "context_window" => serde_json::json!(ExceptionCheckers::context_window(text)),
            "body_code" => get_body_error_code(text)
                .map_or(serde_json::Value::Null, |code| serde_json::json!(code)),
            "message" => return message(input),
            "status" => return status(input),
            _ => serde_json::Value::Null,
        };
        Ok(output)
    }
}

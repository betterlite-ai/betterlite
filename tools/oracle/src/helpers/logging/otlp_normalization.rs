use serde_json::{Value, json};

mod tests;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let object = input.as_object().ok_or_else(invalid_input)?;
        let (operation, body) = object.iter().next().ok_or_else(invalid_input)?;
        dispatch(operation, body)
    }
}

fn invalid_input() -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: "input must name one operation object".to_owned(),
    };
    Box::new(error)
}

fn dispatch(operation: &str, body: &Value) -> crate::helpers::HelperResult {
    match operation {
        "otlp_truncate" => Ok(json!(betterlite_logging::otlp_normalize::truncate(
            body.get("value")
                .and_then(Value::as_str)
                .unwrap_or_default()
        ))),
        "otlp_elision" => Ok(json!(betterlite_logging::otlp_payload::truncate_payload(
            body.get("value")
                .and_then(Value::as_str)
                .unwrap_or_default()
        ))),
        "otlp_content_text" => Ok(json!(betterlite_logging::message_content::content_text(
            body.get("content").unwrap_or(&Value::Null)
        ))),
        "otlp_ui_content" => ui_content(body),
        "otlp_response_encoding" => Ok(response_encoding(body)),
        _ => <Op as crate::helpers::Helper>::not_built("otlp_normalization"),
    }
}

fn ui_content(body: &Value) -> crate::helpers::HelperResult {
    let content = betterlite_logging::ui_content::to_ui_content(
        body.get("value")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    serde_json::to_value(content).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "Json".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

fn response_encoding(body: &Value) -> Value {
    let content_type = body.get("content_type").and_then(Value::as_str);
    let error = body.get("error").and_then(Value::as_str);
    let (encoded, media_type) =
        betterlite_logging::otlp_response::encode_otlp_response(content_type, error);
    let bytes = if media_type == "application/json" {
        error.map_or(encoded, |message| {
            format!(
                "{{\"message\": {}}}",
                serde_json::to_string(message).unwrap_or_default()
            )
            .into_bytes()
        })
    } else {
        Vec::new()
    };
    json!({"body": bytes, "content_type": media_type})
}

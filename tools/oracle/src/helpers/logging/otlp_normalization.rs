use serde_json::{Value, json};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let object = input.as_object().ok_or_else(|| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "input must be an object".to_owned(),
            })
        })?;
        let (operation, body) = object.iter().next().ok_or_else(|| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "input must name an operation".to_owned(),
            })
        })?;
        let output = match operation.as_str() {
            "otlp_truncate" => {
                let value = body
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                json!(betterlite_logging::otlp_normalize::truncate(value))
            }
            "otlp_elision" => {
                let value = body
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                json!(betterlite_logging::otlp_normalize::truncate_payload(value))
            }
            "otlp_content_text" => json!(betterlite_logging::message_content::content_text(
                body.get("content").unwrap_or(&Value::Null)
            )),
            "otlp_ui_content" => {
                serde_json::to_value(betterlite_logging::ui_content::to_ui_content(
                    body.get("value")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ))
                .map_err(|error| {
                    Box::new(crate::helpers::HelperError {
                        class: "Json".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: error.to_string(),
                    })
                })?
            }
            "otlp_response_encoding" => {
                let content_type = body.get("content_type").and_then(Value::as_str);
                let error = body.get("error").and_then(Value::as_str);
                let (bytes, media_type) =
                    betterlite_logging::otlp_response::encode_otlp_response(content_type, error);
                json!({"body": bytes, "content_type": media_type})
            }
            _ => return Self::not_built("otlp_normalization"),
        };
        Ok(output)
    }
}

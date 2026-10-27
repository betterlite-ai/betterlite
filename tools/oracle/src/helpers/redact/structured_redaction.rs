pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let object = input
            .as_object()
            .ok_or_else(|| invalid("structured_redaction input must be an object"))?;
        let function = object
            .get("function")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| invalid("structured_redaction function must be a string"))?;
        let data = object.get("data").unwrap_or(&serde_json::Value::Null);
        let result = match function {
            "mask_sensitive_structure" => {
                betterlite_redact::structured::mask_sensitive_structure(data)
            }
            "mask_credentials_in_payload" => {
                betterlite_redact::structured::mask_credentials_in_payload(data)
            }
            "redact_credentials_in_payload" => {
                betterlite_redact::structured::redact::redact_credentials_in_payload(data)
            }
            "mask_sensitive_keys" => mask_keys(object, data)?,
            _ => return Err(invalid("unknown structured_redaction function")),
        };
        Ok(result)
    }
}

fn mask_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    data: &serde_json::Value,
) -> crate::helpers::HelperResult {
    let fields = object
        .get("sensitive_fields")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| invalid("sensitive_fields must be an array"))?;
    let field_names = fields
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned)
        .collect();
    let map = data
        .as_object()
        .ok_or_else(|| invalid("data must be an object"))?;
    let masked = betterlite_redact::structured::redact::mask_sensitive_keys(map, &field_names);
    Ok(serde_json::Value::Object(masked))
}

fn invalid(message: &str) -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    };
    Box::new(error)
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let object = input.as_object().ok_or_else(|| Box::new(crate::helpers::HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "structured_redaction input must be an object".to_owned(),
        }))?;
        let function = object.get("function").and_then(serde_json::Value::as_str).ok_or_else(|| Box::new(crate::helpers::HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "structured_redaction function must be a string".to_owned(),
        }))?;
        let data = object.get("data").unwrap_or(&serde_json::Value::Null);
        let result = match function {
            "mask_sensitive_structure" => betterlite_redact::structured::mask_sensitive_structure(data),
            "mask_credentials_in_payload" => betterlite_redact::structured::mask_credentials_in_payload(data),
            "redact_credentials_in_payload" => betterlite_redact::structured::redact::redact_credentials_in_payload(data),
            "mask_sensitive_keys" => {
                let fields = object.get("sensitive_fields").and_then(serde_json::Value::as_array).ok_or_else(|| Box::new(crate::helpers::HelperError {
                    class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None,
                    message: "sensitive_fields must be an array".to_owned(),
                }))?;
                let field_names = fields.iter().filter_map(serde_json::Value::as_str).map(str::to_owned).collect();
                let map = data.as_object().ok_or_else(|| Box::new(crate::helpers::HelperError {
                    class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None,
                    message: "data must be an object".to_owned(),
                }))?;
                serde_json::Value::Object(betterlite_redact::structured::redact::mask_sensitive_keys(map, &field_names))
            }
            _ => return Err(Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None,
                message: "unknown structured_redaction function".to_owned(),
            })),
        };
        Ok(result)
    }
}

use betterlite_models::decisions::Json;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let typed_input: Json = serde_json::from_value(input.clone()).map_err(|error| {
            Box::new(crate::compare::helper::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let Json::Object(fields) = typed_input else {
            return Err(Box::new(crate::compare::helper::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "input must be an object".to_owned(),
            }));
        };
        let headers = match fields.get("headers") {
            Some(Json::Object(fields)) => Some(fields),
            _ => None,
        };
        let dynamic = fields.get("dynamic");
        let global = matches!(fields.get("global"), Some(Json::Bool(true)));
        let should_redact = betterlite_logging::logging_privacy::should_redact_message_logging(
            fields.get("metadata"),
            headers,
            dynamic,
            global,
        );
        let snapshot = match fields.get("payload") {
            Some(payload) if should_redact => {
                betterlite_logging::logging_privacy::redacted_snapshot(payload).map_err(
                    |error| {
                        Box::new(crate::compare::helper::HelperError {
                            class: "RedactionError".to_owned(),
                            status: None,
                            error_type: None,
                            param: None,
                            code: None,
                            message: error.to_string(),
                        })
                    },
                )?
            }
            Some(payload) => payload.clone(),
            None => Json::Null,
        };
        let key_info = match fields.get("key_info") {
            Some(value) => betterlite_logging::logging_privacy::redact_user_api_key_info(
                value,
                matches!(fields.get("redact_key_info"), Some(Json::Bool(true))),
            ),
            None => Json::Null,
        };
        Ok(
            serde_json::json!({"should_redact":should_redact,"snapshot":snapshot,"key_info":key_info}),
        )
    }
}

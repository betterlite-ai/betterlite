use betterlite_models::decisions::Json;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let fields = input_fields(input)?;
        let headers = header_fields(&fields);
        let dynamic = fields.get("dynamic");
        let global = matches!(fields.get("global"), Some(Json::Bool(true)));
        let should_redact = betterlite_logging::logging_privacy::should_redact_message_logging(
            fields.get("metadata"),
            headers,
            dynamic,
            global,
        );
        let snapshot = result_snapshot(fields.get("payload"), should_redact)?;
        let key_info = fields.get("key_info").map_or_else(
            || Json::Null,
            |value| {
                betterlite_logging::logging_privacy::redact_user_api_key_info(
                    value,
                    matches!(fields.get("redact_key_info"), Some(Json::Bool(true))),
                )
            },
        );
        Ok(
            serde_json::json!({"should_redact":should_redact,"snapshot":snapshot,"key_info":key_info}),
        )
    }
}

fn input_fields(
    input: &serde_json::Value,
) -> Result<indexmap::IndexMap<String, Json>, Box<crate::compare::helper::HelperError>> {
    let typed_input: Json =
        serde_json::from_value(input.clone()).map_err(|error| invalid_input(error.to_string()))?;
    match typed_input {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(invalid_input("input must be an object".to_owned()))
        }
    }
}

fn invalid_input(message: String) -> Box<crate::compare::helper::HelperError> {
    Box::new(crate::compare::helper::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn header_fields(
    fields: &indexmap::IndexMap<String, Json>,
) -> Option<&indexmap::IndexMap<String, Json>> {
    match fields.get("headers") {
        Some(Json::Object(header_fields)) => Some(header_fields),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

fn result_snapshot(
    payload_option: Option<&Json>,
    should_redact: bool,
) -> Result<Json, Box<crate::compare::helper::HelperError>> {
    match (payload_option, should_redact) {
        (Some(payload_json), true) => {
            betterlite_logging::logging_privacy::redacted_snapshot(payload_json).map_err(|error| {
                Box::new(crate::compare::helper::HelperError {
                    class: "RedactionError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })
        }
        (Some(payload_json), false) => Ok(payload_json.clone()),
        (None, _) => Ok(Json::Null),
    }
}

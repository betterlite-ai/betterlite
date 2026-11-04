pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let headers = input.get("headers").and_then(serde_json::Value::as_object);
        let dynamic = input.get("dynamic");
        let global = input
            .get("global")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let should_redact = betterlite_logging::logging_privacy::should_redact_message_logging(
            input.get("metadata"),
            headers,
            dynamic,
            global,
        );
        let snapshot = input
            .get("payload")
            .map(|payload| {
                if should_redact {
                    betterlite_logging::logging_privacy::redacted_snapshot(payload)
                } else {
                    payload.clone()
                }
            })
            .unwrap_or(serde_json::Value::Null);
        let key_info = input
            .get("key_info")
            .map(|value| {
                betterlite_logging::logging_privacy::redact_user_api_key_info(
                    value,
                    input
                        .get("redact_key_info")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                )
            })
            .unwrap_or(serde_json::Value::Null);
        Ok(
            serde_json::json!({"should_redact":should_redact,"snapshot":snapshot,"key_info":key_info}),
        )
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        use betterlite_logging::otel::model::payloads::{helpers, sanitize_event_metadata};
        let function = input
            .get("function")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let result = match function {
            "cache_token_value" => helpers::cache_token_value(
                input
                    .get("values")
                    .and_then(serde_json::Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            )
            .map_or(serde_json::Value::Null, serde_json::Value::from),
            "guardrail_mode_str" => {
                helpers::guardrail_mode_str(input.get("value").unwrap_or(&serde_json::Value::Null))
                    .map_or(serde_json::Value::Null, serde_json::Value::String)
            }
            "total_masked_entities" => helpers::total_masked_entities(
                input.get("value").unwrap_or(&serde_json::Value::Null),
            )
            .map_or(serde_json::Value::Null, serde_json::Value::from),
            "dicts" => serde_json::json!(helpers::dicts(
                input.get("value").unwrap_or(&serde_json::Value::Null)
            )),
            "json_or_none" => {
                helpers::json_or_none(input.get("value").unwrap_or(&serde_json::Value::Null))
                    .map_or(serde_json::Value::Null, serde_json::Value::String)
            }
            "sanitize_event_metadata" => {
                let values = input
                    .get("value")
                    .and_then(serde_json::Value::as_object)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
                serde_json::json!(sanitize_event_metadata::sanitize_event_metadata(&values))
            }
            _ => {
                return Err(Box::new(crate::helpers::HelperError {
                    class: "InvalidInput".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: format!("unknown B3843 function {function}"),
                }));
            }
        };
        Ok(result)
    }
}

mod tests;

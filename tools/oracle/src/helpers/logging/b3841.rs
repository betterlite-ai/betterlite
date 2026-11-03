use betterlite_logging::otel::model::baggage::promoted_baggage;
use betterlite_logging::otel::model::metadata::identity::{RequestContext, RequestIdentity};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use serde_json::{Value, json};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.get("function").and_then(Value::as_str) {
            Some("identity") => {
                let payload = input
                    .get("payload")
                    .ok_or_else(|| invalid("payload is required"))?;
                let payload = serde_json::from_value::<OtelModelValue>(payload.clone())
                    .map_err(|error| invalid(&error.to_string()))?;
                serde_json::to_value(RequestIdentity::from_payload(&payload))
                    .map_err(|error| invalid(&error.to_string()))
            }
            Some("context") => {
                let payload = input
                    .get("payload")
                    .ok_or_else(|| invalid("payload is required"))?;
                let payload = serde_json::from_value::<OtelModelValue>(payload.clone())
                    .map_err(|error| invalid(&error.to_string()))?;
                serde_json::to_value(RequestContext::from_standard_logging_payload(&payload))
                    .map_err(|error| invalid(&error.to_string()))
            }
            Some("session") => session(input),
            Some("flatten") => {
                let metadata = input.get("metadata").cloned().unwrap_or_else(|| json!({}));
                let metadata = serde_json::from_value::<OtelModelValue>(metadata)
                    .map_err(|error| invalid(&error.to_string()))?;
                let flattened = match &metadata {
                    OtelModelValue::Object(values) => Some(values),
                    _ => None,
                };
                Ok(json!(
                    betterlite_logging::otel::model::metadata::identity::flatten_metadata(
                        flattened
                    )
                ))
            }
            Some("baggage") => baggage(input),
            _ => Err(invalid("unknown b3841 function")),
        }
    }
}

#[path = "b3841/tests.rs"]
mod tests;

fn session(input: &Value) -> crate::helpers::HelperResult {
    let kwargs = input
        .get("kwargs")
        .ok_or_else(|| invalid("kwargs is required"))?;
    let kwargs = serde_json::from_value::<OtelModelValue>(kwargs.clone())
        .map_err(|error| invalid(&error.to_string()))?;
    let trace = betterlite_logging::otel::model::trace_controls::caller_trace_controls(&kwargs);
    Ok(
        json!({"session_id": betterlite_logging::otel::model::metadata::callback::caller_session_id(&kwargs, &trace)}),
    )
}

fn baggage(input: &Value) -> crate::helpers::HelperResult {
    let identity = input
        .get("identity")
        .ok_or_else(|| invalid("identity is required"))?;
    let identity: RequestIdentity =
        serde_json::from_value(identity.clone()).map_err(|error| invalid(&error.to_string()))?;
    let promoted: Vec<String> = input
        .get("promoted_keys")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let metadata: Vec<String> = input
        .get("metadata_keys")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let team_metadata: Vec<String> = input
        .get("team_metadata_keys")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    Ok(json!(promoted_baggage(
        &identity,
        input.get("request_model").and_then(Value::as_str),
        &promoted,
        &metadata,
        &team_metadata
    )))
}

fn invalid(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

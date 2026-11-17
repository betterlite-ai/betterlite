use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;
use serde_json::{Value, json};

// LiteLLM writes the billed model, provider and cost onto the logging object, and the oracle compares those details.
pub(super) fn billing_view(logging_obj: &StreamLoggingObject) -> Result<Value, Box<HelperError>> {
    let Some(details) = logging_obj.read(&mut |state| state.model_call_details.clone()) else {
        return Err(Box::new(HelperError {
            class: "LoggingState".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "the logging object holds no state".to_owned(),
        }));
    };
    Ok(json!({
        "model": entry_text(&details, "model"),
        "custom_llm_provider": entry_text(&details, "custom_llm_provider"),
        "response_cost": entry_text(&details, "response_cost"),
    }))
}

fn entry_text(fields: &IndexMap<String, Json>, name: &str) -> Value {
    let Some(value) = fields.get(name) else {
        return Value::Null;
    };
    match value {
        Json::Text(text) => Value::String(text.clone()),
        Json::Number(number) => Value::String(number.to_string()),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => Value::Null,
    }
}

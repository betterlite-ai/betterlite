pub mod cost_dimension;
pub mod datadog_budget_reset_decoder;
pub mod guardrail_redact;
pub mod latency;
pub mod message_redact;
pub mod metadata;
pub mod redaction_decision;
pub mod router_span;
pub mod spend;
pub mod stream;

mod json_number;
mod tests;

use betterlite_models::decisions::Json;

use crate::datadog_llm_obs::values::Object;

// A span is assembled from typed records that Serde already knows how to write; a record it cannot is reported
#[derive(Debug, thiserror::Error)]
pub enum RecordEncodingError {
    #[error("the {record} record could not be encoded")]
    Encode {
        record: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the {record} record is not a JSON object")]
    Shape { record: &'static str },
}

pub fn record_object<Record>(
    record: &'static str,
    value: &Record,
) -> Result<Object, RecordEncodingError>
where
    Record: serde::Serialize,
{
    let encoded = serde_json::to_string(value)
        .map_err(|source| RecordEncodingError::Encode { record, source })?;
    match serde_json::from_str::<Json>(&encoded) {
        Ok(Json::Object(fields)) => Ok(fields),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(RecordEncodingError::Shape { record })
        }
        Err(source) => Err(RecordEncodingError::Encode { record, source }),
    }
}

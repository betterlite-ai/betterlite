use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::Usage;

use super::Event;
use super::error::RealtimeCostError;
use super::usage::aggregate_usage;

// Ports collect_usage_from_realtime_stream_results: response.done keeps its required type and response keys.
pub fn collect_usage(results: &[Event]) -> Result<Vec<Usage>, RealtimeCostError> {
    let mut usages = Vec::new();
    for event in results {
        let Some(kind) = event.get("type") else {
            return Err(RealtimeCostError::MissingField { field: "type" });
        };
        if !matches!(kind, Json::Text(event_type) if event_type == "response.done") {
            continue;
        }
        let Some(response_value) = event.get("response") else {
            return Err(RealtimeCostError::MissingField { field: "response" });
        };
        let Json::Object(response) = response_value else {
            return Err(RealtimeCostError::Shape {
                field: "response",
                expected: "an object",
            });
        };
        usages.push(aggregate_usage(response.get("usage"))?);
    }
    Ok(usages)
}

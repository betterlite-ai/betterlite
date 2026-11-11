use betterlite_models::decisions::Json;
use chrono::DateTime;
use chrono::FixedOffset;

#[must_use]
pub fn get_logging_id(start_time: DateTime<FixedOffset>, response: &Json) -> Option<String> {
    let Json::Object(fields) = response else {
        return None;
    };
    let Some(Json::Text(id)) = fields.get("id") else {
        return None;
    };
    let micros = start_time.timestamp_subsec_micros();
    Some(format!(
        "time-{}-{:06}_{id}",
        start_time.format("%H-%M-%S"),
        micros
    ))
}

use betterlite_db::health_check_rows::HealthCheckRow;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, Utc};
use serde_json::Number;

// _convert_health_check_to_dict: the twelve stored fields in LiteLLM's key order, and no updated_at
#[must_use]
pub fn convert_health_check(check: &HealthCheckRow) -> JsonObject {
    [
        ("health_check_id", Json::Text(check.health_check_id.clone())),
        ("model_name", Json::Text(check.model_name.clone())),
        (
            "model_id",
            check.model_id.clone().map_or(Json::Null, Json::Text),
        ),
        ("status", Json::Text(check.status.clone())),
        (
            "healthy_count",
            Json::Number(Number::from(check.healthy_count)),
        ),
        (
            "unhealthy_count",
            Json::Number(Number::from(check.unhealthy_count)),
        ),
        (
            "error_message",
            check.error_message.clone().map_or(Json::Null, Json::Text),
        ),
        ("response_time_ms", response_time(check.response_time_ms)),
        ("details", check.details.clone().unwrap_or(Json::Null)),
        (
            "checked_by",
            check.checked_by.clone().map_or(Json::Null, Json::Text),
        ),
        ("checked_at", Json::Text(isoformat(check.checked_at))),
        ("created_at", Json::Text(isoformat(check.created_at))),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect()
}

fn response_time(value: Option<f64>) -> Json {
    value
        .and_then(Number::from_f64)
        .map_or(Json::Null, Json::Number)
}

// datetime.isoformat() for a UTC value: microseconds only when they are not zero, then the UTC offset
#[must_use]
pub fn isoformat(value: DateTime<Utc>) -> String {
    let seconds = value.format("%Y-%m-%dT%H:%M:%S");
    let micros = value.timestamp_subsec_micros();
    if micros == 0 {
        return format!("{seconds}+00:00");
    }
    format!("{seconds}.{micros:06}+00:00")
}

use chrono::{DateTime, Utc};

#[must_use]
pub fn generate_success_object_name(date: &str, response_id: &str) -> String {
    format!("{date}/{response_id}")
}

#[must_use]
pub fn generate_failure_object_name(date: &str, uuid: &str) -> String {
    format!("{date}/failure-{uuid}")
}

#[must_use]
pub fn generate_batch_object_name(date: &str, batch_id: &str) -> String {
    format!("{date}/batch-{batch_id}.ndjson")
}

#[must_use]
pub fn get_object_date(datetime: DateTime<Utc>) -> String {
    datetime.format("%Y-%m-%d").to_string()
}

mod tests;

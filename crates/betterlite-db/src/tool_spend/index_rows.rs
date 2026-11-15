use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ToolIndexRow {
    pub request_id: String,
    pub tool_name: String,
    pub start_time: String,
    #[serde(skip)]
    pub instant: DateTime<Utc>,
}

impl ToolIndexRow {
    #[must_use]
    pub fn new(request_id: String, tool_name: String, instant: DateTime<Utc>) -> Self {
        Self {
            request_id,
            tool_name,
            start_time: datetime_text(instant),
            instant,
        }
    }
}

// json.dumps(row, default=str) writes str(datetime): a space separator, six digits of microseconds, the offset
fn datetime_text(instant: DateTime<Utc>) -> String {
    if instant.timestamp_subsec_micros() == 0 {
        return instant.format("%Y-%m-%d %H:%M:%S+00:00").to_string();
    }
    instant.format("%Y-%m-%d %H:%M:%S%.6f+00:00").to_string()
}

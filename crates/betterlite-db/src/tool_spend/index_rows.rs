use chrono::{DateTime, Utc};
use serde::Serializer;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ToolIndexRow {
    pub request_id: String,
    pub tool_name: String,
    #[serde(serialize_with = "python_timestamp")]
    pub start_time: DateTime<Utc>,
}

// json.dumps(row, default=str) writes str(datetime): a space separator, six digits of microseconds, the offset
#[must_use]
pub fn python_timestamp<S>(start_time: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let seconds = start_time.naive_utc().format("%Y-%m-%d %H:%M:%S");
    let micros = start_time.timestamp_subsec_micros();
    let text = if micros == 0 {
        format!("{seconds}+00:00")
    } else {
        format!("{seconds}.{micros:06}+00:00")
    };
    serializer.serialize_str(&text)
}

use chrono::{DateTime, Utc};

// the two v2 window keys: "{model_id}:{deployment_name}:{rpm|tpm}:{hour}-{minute}"
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowKeys {
    pub rpm: String,
    pub tpm: String,
}

// get_utc_datetime().strftime("%H-%M"), in utc regardless of the system clock
#[must_use]
pub fn window_keys(model_id: &str, model: &str, now: DateTime<Utc>) -> WindowKeys {
    let minute = now.format("%H-%M").to_string();
    let rpm = format!("{model_id}:{model}:rpm:{minute}");
    let tpm = format!("{model_id}:{model}:tpm:{minute}");
    WindowKeys { rpm, tpm }
}

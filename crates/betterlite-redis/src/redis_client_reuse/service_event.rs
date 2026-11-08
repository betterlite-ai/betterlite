use std::time::Duration;

pub struct ServiceEvent {
    pub call_type: &'static str,
    pub duration: Duration,
    pub key: Option<String>,
}

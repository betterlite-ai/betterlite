use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct RecordTimes {
    pub(super) start_time: f64,
    pub(super) end_time: f64,
    pub(super) duration: f64,
}

// LiteLLM stamps start_time, end_time and duration around every hook call it logs
pub(super) fn between(started_at: SystemTime, finished_at: SystemTime) -> RecordTimes {
    RecordTimes {
        start_time: started_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64(),
        end_time: finished_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64(),
        duration: finished_at
            .duration_since(started_at)
            .unwrap_or_default()
            .as_secs_f64(),
    }
}

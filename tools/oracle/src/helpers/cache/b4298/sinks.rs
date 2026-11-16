use std::sync::{Mutex, PoisonError};

use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_redact::structured::Payload;
#[derive(Default)]
pub(crate) struct RecordingLogger {
    pub(crate) records: Mutex<Vec<(String, String)>>,
}

impl RecordingLogger {
    pub(crate) fn matching(&self, level: &str, marker: &str) -> usize {
        let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        records
            .iter()
            .filter(|(recorded, text)| recorded == level && text.contains(marker))
            .count()
    }
}

impl LoggerSink for RecordingLogger {
    fn debug(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(("DEBUG".to_owned(), payload_text(payload)));
        true
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(("ERROR".to_owned(), payload_text(payload)));
        true
    }

    fn debug_enabled(&self) -> bool {
        true
    }
}

fn payload_text(payload: &Payload) -> String {
    match payload {
        Payload::String(text) => text.clone(),
        Payload::Null
        | Payload::Bool(_)
        | Payload::Number(_)
        | Payload::Array(_)
        | Payload::Object(_) => String::new(),
    }
}

pub(crate) struct RecordingConsole;

impl ConsoleSink for RecordingConsole {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

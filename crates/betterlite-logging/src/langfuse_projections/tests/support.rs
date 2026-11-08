#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::input_output::ContentRequest;

pub(super) fn value(text: &str) -> Json {
    serde_json::from_str(text).expect("valid fixture JSON")
}

pub(super) fn content_request(call_type: &'static str) -> ContentRequest<'static> {
    ContentRequest {
        call_type: Some(call_type),
        input: None,
        level: "DEFAULT",
        status_message: None,
    }
}

pub(super) fn prompt() -> IndexMap<String, Json> {
    let mut prompt = IndexMap::new();
    prompt.insert(
        "messages".to_owned(),
        value(r#"[{"role": "user", "content": "hi"}]"#),
    );
    prompt
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("captured logs are UTF-8")
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Captured {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}

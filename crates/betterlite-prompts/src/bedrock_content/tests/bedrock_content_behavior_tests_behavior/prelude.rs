use std::io;
use std::sync::{Arc, Mutex};

use tracing::subscriber::{DefaultGuard, set_default};

use crate::arguments::json::{Json, Object};

pub(super) fn json(raw: &str) -> Json {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn object(raw: &str) -> Object {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn field<'input>(value: &'input Json, key: &str) -> &'input Json {
    value.as_object().unwrap().get(key).unwrap()
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

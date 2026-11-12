use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

#[derive(Clone, Default)]
pub(super) struct Captured {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl Captured {
    pub(super) fn fields(&self, level: &str) -> Vec<Value> {
        let bytes = self.bytes.lock().unwrap_or_else(PoisonError::into_inner);
        bytes
            .split(|byte| *byte == b'\n')
            .filter_map(|line| {
                let event = serde_json::from_slice::<Value>(line).ok()?;
                let fields = event.get("fields")?;
                (event.get("level")? == level && fields.get("subject")?.is_string())
                    .then(|| fields.clone())
            })
            .collect()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Captured {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let subscriber = fmt()
        .json()
        .without_time()
        .with_target(false)
        .with_max_level(Level::DEBUG)
        .with_writer(captured.clone())
        .finish();
    (captured, set_default(subscriber))
}

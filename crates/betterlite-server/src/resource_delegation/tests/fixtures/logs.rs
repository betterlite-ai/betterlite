use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::Level;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;

#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(crate) fn capture() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let guard = tracing_subscriber::fmt()
            .with_max_level(Level::WARN)
            .with_writer(logs.clone())
            .with_ansi(false)
            .set_default();
        (logs, guard)
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the captured log output is UTF-8")
    }
}

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut logged = self
            .0
            .lock()
            .map_err(|_poisoned| io::Error::other("log buffer poisoned"))?;
        logged.extend_from_slice(buf);
        drop(logged);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

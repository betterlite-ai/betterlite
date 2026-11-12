use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;

use super::fakes::lock;

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(crate) fn contents(&self) -> String {
        let bytes = lock(&self.0).clone();
        String::from_utf8(bytes).expect("the log writer records UTF-8")
    }

    pub(crate) fn has(&self, level: &str, needle: &str) -> bool {
        self.contents()
            .lines()
            .any(|line| line.contains(level) && line.contains(needle))
    }

    pub(crate) fn count(&self, needle: &str) -> usize {
        self.contents()
            .lines()
            .filter(|line| line.contains(needle))
            .count()
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        lock(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (CapturedLogs, impl tracing::Subscriber + Send + Sync) {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    (logs, subscriber)
}

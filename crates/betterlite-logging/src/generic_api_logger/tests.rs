#![cfg(test)]

use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};

use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use std::collections::BTreeMap;

use super::generic_settings::GenericApiArgs;
pub(crate) const ENDPOINT: &str = "https://hook.test/generic";
pub(crate) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn args() -> GenericApiArgs {
    GenericApiArgs {
        callback_name: None,
        endpoint: Some(ENDPOINT.to_owned()),
        event_types: None,
        headers: None,
        log_format: None,
        max_retries: None,
        retry_delay: None,
        timeout: None,
    }
}
#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

impl LogBuffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("utf-8 log text")
    }
}
pub(crate) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}

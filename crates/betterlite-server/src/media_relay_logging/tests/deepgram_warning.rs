#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::media_relay_logging::deepgram::DeepgramListenRequest;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::deepgram::deepgram_listen_passthrough_handler;
use super::support::{Registry, kwargs, logging_object, lookup, messages, registry};

const CATALOG: &[(&str, &str)] = &[(
    "deepgram/streaming/nova-9",
    r#"{"litellm_provider": "deepgram", "input_cost_per_second": 0.00008}"#,
)];

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl io::Write for LogBuffer {
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

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

fn listen(registry: &Registry) {
    let logging_obj = logging_object();
    let frames = messages(r#"[{"type": "Metadata", "duration": 2}]"#);
    let caller = kwargs(&[]);
    let request = DeepgramListenRequest {
        upstream_url: "https://api.deepgram.com/v1/listen?model=nova-9",
        messages: &frames,
        kwargs: &caller,
        catalog: &registry.catalog,
        lookup: &registry.lookup,
    };
    deepgram_listen_passthrough_handler(&logging_obj, &request).expect("the handler runs");
}

#[test]
fn deepgram_listen_warns_when_the_registry_lacks_the_base_entry() {
    let (logs, _guard) = capture_logs();

    listen(&registry(&[]));

    let contents = logs.contents();
    assert!(contents.contains("no registry entry"));
    assert!(contents.contains("deepgram/streaming/nova-9"));
}

#[test]
fn deepgram_listen_stays_quiet_when_the_base_entry_exists() {
    let (logs, _guard) = capture_logs();

    listen(&registry(CATALOG));

    assert!(!logs.contents().contains("no registry entry"));
}

#[test]
fn deepgram_listen_stays_quiet_when_only_the_base_rate_is_unreadable() {
    let (logs, _guard) = capture_logs();
    let unreadable = Registry {
        catalog: registry(CATALOG).catalog,
        lookup: lookup(&[(
            "deepgram/streaming/nova-9",
            r#"{"litellm_provider": "deepgram", "input_cost_per_second": [1]}"#,
        )]),
    };

    listen(&unreadable);

    assert!(!logs.contents().contains("no registry entry"));
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::registry::catalog::{CatalogError, CatalogInputs};
use crate::sqs_logging::tests::support::{event, payload};

use super::build;

fn inputs(source: &str) -> CatalogInputs {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("the config loads");
    let callback_settings = config.sections.callback_settings.callback_settings.clone();
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("the HTTP client builds"),
        env: Arc::new(BTreeMap::<String, String>::new()),
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("the log is UTF-8")
    }
}

impl Write for Buffer {
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

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

fn subscribe(buffer: &Buffer) -> DefaultGuard {
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::WARN)
        .with_writer(buffer.clone())
        .finish();
    set_default(subscriber)
}

#[tokio::test]
async fn callback_logger_builds_the_s3_logger() {
    let inputs = inputs(
        "callback_settings:\n  s3:\n    s3_bucket_name: logs\n    s3_region_name: us-east-1\n",
    );
    let logger = build(&inputs).expect("the logger builds");
    assert_eq!(logger.callback_name(), "s3");
}

#[tokio::test]
async fn callback_logger_builds_without_a_bucket() {
    let inputs = inputs("");
    let logger = build(&inputs).expect("the logger builds");
    assert_eq!(logger.callback_name(), "s3");
}

#[tokio::test]
async fn callback_logger_refuses_an_unsupported_setting() {
    let inputs = inputs("callback_settings:\n  s3:\n    s3_api_version: v1\n");
    let error = build(&inputs).err().expect("the setting is refused");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("s3_api_version")
    ));
}

#[tokio::test]
async fn callback_logger_refuses_an_invalid_region() {
    let inputs = inputs(
        "callback_settings:\n  s3:\n    s3_bucket_name: logs\n    s3_region_name: US_EAST\n",
    );
    let error = build(&inputs).err().expect("the region is refused");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("US_EAST")
    ));
}

#[tokio::test]
async fn callback_logger_reads_the_team_prefix_settings() {
    let inputs = inputs(
        "callback_settings:\n  s3:\n    s3_bucket_name: logs\n    s3_region_name: us-east-1\n    s3_use_team_prefix: true\n    enable_preview_features: true\n",
    );
    let logger = build(&inputs).expect("the logger builds");
    let buffer = Buffer::default();
    let guard = subscribe(&buffer);
    let mut standard = payload();
    standard.metadata.user_api_key_team_alias = Some("team".to_owned());
    logger
        .async_log_success_event(event(standard))
        .await
        .expect("the event is accepted");
    drop(guard);
    assert!(
        buffer
            .contents()
            .contains("team/2009-02-13/time-23-31-30-000000_sqs-health-check.json"),
        "{}",
        buffer.contents()
    );
}

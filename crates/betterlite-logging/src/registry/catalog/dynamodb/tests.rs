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

fn inputs(source: &str, variables: &[(&str, &str)]) -> CatalogInputs {
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
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("the HTTP client builds"),
        env: Arc::new(environment),
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn configured() -> CatalogInputs {
    inputs(
        "callback_settings:\n  dynamodb:\n    dynamodb_table_name: logs\n",
        &[("AWS_REGION_NAME", "us-east-1")],
    )
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
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    set_default(subscriber)
}

#[tokio::test]
async fn callback_logger_builds_the_dynamodb_logger() {
    let inputs = configured();
    let logger = build(&inputs).expect("the logger builds");
    assert_eq!(logger.callback_name(), "dynamodb");
}

#[tokio::test]
async fn callback_logger_reads_the_table_and_region_settings() {
    let inputs = configured();
    let logger = build(&inputs).expect("the logger builds");
    let buffer = Buffer::default();
    let guard = subscribe(&buffer);
    logger
        .async_log_success_event(event(payload()))
        .await
        .expect("the event is accepted");
    drop(guard);
    let logs = buffer.contents();
    assert!(logs.contains("table=logs"), "{logs}");
    assert!(logs.contains("region=us-east-1"), "{logs}");
}

#[tokio::test]
async fn callback_logger_requires_the_table_name() {
    let inputs = inputs("", &[("AWS_REGION_NAME", "us-east-1")]);
    let error = build(&inputs).err().expect("the table is missing");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("dynamodb_table_name")
    ));
}

#[tokio::test]
async fn callback_logger_requires_the_region_variable() {
    let inputs = inputs(
        "callback_settings:\n  dynamodb:\n    dynamodb_table_name: logs\n",
        &[],
    );
    let error = build(&inputs).err().expect("the region is missing");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("AWS_REGION_NAME")
    ));
}

#[tokio::test]
async fn callback_logger_refuses_an_unknown_setting() {
    let inputs = inputs(
        "callback_settings:\n  dynamodb:\n    dynamodb_table_name: logs\n    dynamodb_typo: 1\n",
        &[("AWS_REGION_NAME", "us-east-1")],
    );
    let error = build(&inputs).err().expect("the setting is refused");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("dynamodb_typo")
    ));
}

#![cfg(test)]

use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use indexmap::IndexMap;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::callback_logger::CallbackLogger as _;
use crate::sqs_logging::tests::support::{event, payload};

use super::DynamoDbLogger;
use super::client::DynamoClient;
use super::settings::Settings;

#[derive(Debug, thiserror::Error)]
#[error("the fake DynamoDB client failed")]
struct FakeError;

type Puts = Arc<Mutex<Vec<(String, IndexMap<String, String>)>>>;

#[derive(Debug, Default)]
struct FakeClient {
    puts: Puts,
    failing: bool,
}

#[async_trait]
impl DynamoClient for FakeClient {
    type Error = FakeError;

    async fn put_item(
        &self,
        table: &str,
        item: &IndexMap<String, String>,
    ) -> Result<(), FakeError> {
        self.puts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((table.to_owned(), item.clone()));
        if self.failing {
            return Err(FakeError);
        }
        Ok(())
    }
}

fn settings() -> Settings {
    Settings {
        table_name: "logs".to_owned(),
        region_name: "us-east-1".to_owned(),
    }
}

fn logger(failing: bool) -> (DynamoDbLogger<FakeClient>, Puts) {
    let puts = Arc::new(Mutex::new(Vec::new()));
    let client = FakeClient {
        puts: Arc::clone(&puts),
        failing,
    };
    (
        DynamoDbLogger {
            settings: settings(),
            client,
        },
        puts,
    )
}

fn recorded(puts: &Puts) -> Vec<(String, IndexMap<String, String>)> {
    puts.lock().unwrap_or_else(PoisonError::into_inner).clone()
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

fn subscriber(buffer: &Buffer) -> DefaultGuard {
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    set_default(subscriber)
}

#[tokio::test]
async fn callback_logger_puts_the_stringified_item() {
    let (logger, puts) = logger(false);
    logger
        .async_log_success_event(event(payload()))
        .await
        .expect("the event is accepted");
    let recorded_puts = recorded(&puts);
    assert_eq!(recorded_puts.len(), 1);
    let (table, item) = &recorded_puts[0];
    assert_eq!(table, "logs");
    assert_eq!(item["id"], "sqs-health-check");
    assert_eq!(item["model"], "gpt-3.5-turbo");
    assert_eq!(item["startTime"], "2009-02-13T23:31:30+00:00");
}

#[tokio::test]
async fn callback_logger_skips_a_missing_payload() {
    let (logger, puts) = logger(false);
    logger.log_event(None).await;
    assert!(recorded(&puts).is_empty());
}

#[tokio::test]
async fn callback_logger_swallows_a_failed_put() {
    let (logger, puts) = logger(true);
    let buffer = Buffer::default();
    let guard = subscriber(&buffer);
    logger
        .async_log_success_event(event(payload()))
        .await
        .expect("the callback never propagates a failure");
    drop(guard);
    assert_eq!(recorded(&puts).len(), 1);
    assert!(
        buffer.contents().contains("DynamoDB Layer Error"),
        "{}",
        buffer.contents()
    );
}

#[tokio::test]
async fn callback_logger_reports_the_dynamodb_name() {
    let (logger, _puts) = logger(false);
    assert_eq!(logger.callback_name(), "dynamodb");
}

use std::future::pending;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_mcp::client::cleanup::http_client::CleanupScope;
use betterlite_mcp::client::sessions::error::{ExitFailure, McpFailure};
use betterlite_mcp::client::sessions::factory::{HttpClientConfig, SseRequest};
use betterlite_mcp::client::sessions::negotiate::InitializeRequest;
use betterlite_mcp::client::sessions::runtime::HttpClientHandle;
use betterlite_mcp::client::sessions::transport::StdioServerParameters;
use serde::Deserialize;
use thiserror::Error;
use tokio::time::sleep;

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum TestError {
    #[error("boom: {0}")]
    Boom(&'static str),
    #[error("mcp error {0}")]
    Protocol(McpFailure),
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Step {
    #[default]
    Ok,
    Fail,
    Cancel,
    Timeout,
    RecordedFailure,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InitStep {
    #[default]
    Ok,
    Fail,
    Protocol,
    Timeout,
    StreamFailure,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Plan {
    #[serde(default)]
    pub(crate) transport_enter: Step,
    #[serde(default)]
    pub(crate) transport_exit: Step,
    #[serde(default)]
    pub(crate) session_enter: Step,
    #[serde(default)]
    pub(crate) session_exit: Step,
    #[serde(default)]
    pub(crate) initialize: InitStep,
    #[serde(default = "default_server_version")]
    pub(crate) server_version: String,
    #[serde(default)]
    pub(crate) instructions: Option<String>,
    #[serde(default)]
    pub(crate) http_close: Step,
}

fn default_server_version() -> String {
    "2025-06-18".to_owned()
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            transport_enter: Step::Ok,
            transport_exit: Step::Ok,
            session_enter: Step::Ok,
            session_exit: Step::Ok,
            initialize: InitStep::Ok,
            server_version: default_server_version(),
            instructions: None,
            http_close: Step::Ok,
        }
    }
}

#[derive(Default)]
pub(crate) struct Script {
    pub(crate) calls: Mutex<Vec<String>>,
    pub(crate) requests: Mutex<Vec<InitializeRequest>>,
    pub(crate) stdio: Mutex<Option<StdioServerParameters>>,
    pub(crate) sse: Mutex<Option<SseRequest>>,
    pub(crate) http: Mutex<Vec<HttpClientConfig>>,
    pub(crate) urls: Mutex<Vec<String>>,
}

impl Script {
    pub(crate) fn record(&self, name: &str) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Default)]
pub(crate) struct FakeHttp {
    pub(crate) errors: Arc<Mutex<Vec<Arc<TestError>>>>,
}

impl FakeHttp {
    pub(crate) fn record_error(&self, error: Arc<TestError>) {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(error);
    }
}

impl HttpClientHandle for FakeHttp {
    type Error = TestError;

    fn begin_cleanup(&self, _scope: CleanupScope) {}

    fn finish_cleanup(&self) -> Vec<Arc<TestError>> {
        std::mem::take(&mut *self.errors.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

pub(crate) async fn exit_result(
    step: Step,
    http: Option<&FakeHttp>,
    label: &'static str,
) -> Result<(), ExitFailure<TestError>> {
    match step {
        Step::Ok => Ok(()),
        Step::Cancel => Err(ExitFailure::Cancelled),
        Step::Timeout => {
            sleep(Duration::from_secs(60)).await;
            Ok(())
        }
        Step::Fail => Err(ExitFailure::Failure(Arc::new(TestError::Boom(label)))),
        Step::RecordedFailure => {
            let error = Arc::new(TestError::Boom("recorded cleanup error"));
            if let Some(client) = http {
                client.record_error(Arc::clone(&error));
            }
            Err(ExitFailure::Group(vec![ExitFailure::Failure(error)]))
        }
    }
}

pub(crate) async fn enter_result(step: Step, label: &'static str) -> Result<(), TestError> {
    match step {
        Step::Fail => Err(TestError::Boom(label)),
        Step::Cancel => pending::<Result<(), TestError>>().await,
        Step::Ok | Step::Timeout | Step::RecordedFailure => Ok(()),
    }
}

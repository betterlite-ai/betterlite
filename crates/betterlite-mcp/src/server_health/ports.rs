use std::collections::BTreeMap;
use std::future::Future;
use std::time::{Duration, Instant};

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::openapi_tools::error::OpenApiError;
use crate::server_record::MCPServer;

pub trait ProbeClient: Send + Sync {
    fn get_stream_status(
        &self,
        url: &str,
        accept: &str,
        timeout: Duration,
    ) -> impl Future<Output = Result<(), ProbeError>> + Send;
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ProbeError {
    #[error("the reachability request timed out")]
    Timeout,
    #[error("the reachability request failed ({0})")]
    Failed(String),
}

pub trait SessionProbe: Send + Sync {
    fn run_noop(
        &self,
        server: &MCPServer,
        static_headers: Option<&BTreeMap<String, String>>,
        timeout: Duration,
    ) -> impl Future<Output = Result<Option<String>, SessionError>> + Send;
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SessionError {
    #[error("the MCP session timed out")]
    Timeout,
    #[error("the MCP session was cancelled")]
    Cancelled,
    #[error("{0}")]
    Failed(String),
}

pub trait EnvVarCheck: Send + Sync {
    #[must_use]
    fn references_per_user_env_var(&self, server: &MCPServer) -> bool;
}

pub trait AllowedServers: Send + Sync {
    #[must_use]
    fn allowed_server_ids(&self, auth: Option<&UserAPIKeyAuth>) -> Vec<String>;
}

pub trait Clock: Send + Sync {
    #[must_use]
    fn now_monotonic(&self) -> Duration;
    #[must_use]
    fn now_utc(&self) -> DateTime<Utc>;
}

pub trait SpecLoader: Send + Sync {
    fn load(
        &self,
        spec_path: &str,
        max_bytes: usize,
    ) -> impl Future<Output = Result<(), OpenApiError>> + Send;
}

#[derive(Clone, Copy, Debug)]
pub struct SystemClock {
    start: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now_monotonic(&self) -> Duration {
        self.start.elapsed()
    }

    fn now_utc(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

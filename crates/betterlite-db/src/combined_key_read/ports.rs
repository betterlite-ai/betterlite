use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use opentelemetry::trace::SpanContext;

use crate::auth_reconnect::request::ReconnectRequest;
use crate::auth_reconnect::stale::StaleReadEngine;
use crate::budget_permission_rows::JsonData;
use crate::database_failures::metrics::{ProxySpanResolver, WallClock};
use crate::database_refresh::generation::Observation;
use crate::key_hash::KeyHash;
use crate::key_user_writes::retry::WriteClock;

use super::error::{DatabaseFailure, KeyReadError, ReconnectFailure};
use super::grace::DeprecatedToken;

pub const QUERY_FIRST_METHOD: &str = "query_first";

pub type RowFuture<'lookup> =
    Pin<Box<dyn Future<Output = Result<Option<JsonData>, DatabaseFailure>> + Send + 'lookup>>;

pub type DeprecatedFuture<'lookup> = Pin<
    Box<dyn Future<Output = Result<Option<DeprecatedToken>, DatabaseFailure>> + Send + 'lookup>,
>;

pub type ReconnectFuture<'reconnect> =
    Pin<Box<dyn Future<Output = Result<bool, ReconnectFailure>> + Send + 'reconnect>>;

pub type HookFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

// one attempt at the combined view query on the writer when pinned, or on whichever handle routing selects
pub trait KeyReadDatabase: Send + Sync {
    fn read_row<'lookup>(
        &'lookup self,
        hash: &'lookup KeyHash,
        use_writer: bool,
    ) -> RowFuture<'lookup>;

    fn deprecated_row<'lookup>(
        &'lookup self,
        hash: &'lookup KeyHash,
        now: DateTime<Utc>,
        use_writer: bool,
    ) -> DeprecatedFuture<'lookup>;
}

pub trait PlanReconnect: Send + Sync {
    type Handle: Send + Sync + 'static;

    fn read_target(&self) -> Observation<Self::Handle>;

    fn attempt<'reconnect>(
        &'reconnect self,
        request: &'reconnect ReconnectRequest<'_>,
        stale: Option<&'reconnect StaleReadEngine<Self::Handle>>,
    ) -> ReconnectFuture<'reconnect>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Diagnostic<'text> {
    TokenHash,
    CachedPlan { message: &'text str },
    AttemptFailure { message: &'text str },
    FailureTraceback { message: &'text str },
    Backoff { tries: u32 },
    DeprecatedUsed,
    DeprecatedLookupFailed { message: &'text str },
}

pub trait KeyReadDiagnostics: Send + Sync {
    fn emit(&self, diagnostic: Diagnostic<'_>);
}

#[derive(Debug, Clone)]
pub struct FailureHook {
    pub call_type: &'static str,
    pub message: String,
    pub traceback: String,
    pub duration: Duration,
    // the original typed failure, alongside the redacted message and traceback the hook also carries
    pub failure: KeyReadError,
}

// the source detaches failure_handler from the failed lookup, so the hook is scheduled and never awaited here
pub trait KeyFailureHook: Send + Sync {
    fn schedule(&self, hook: FailureHook) -> HookFuture;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoSpanResolver;

impl ProxySpanResolver for NoSpanResolver {
    fn parent_span(&self, _passed_kwargs: &IndexMap<String, Json>) -> Option<SpanContext> {
        None
    }
}

// one attempt records its own wall clock, so the metric wrapper reads the same clock
pub struct MetricWallClock<'clock>(pub &'clock dyn WriteClock);

impl WallClock for MetricWallClock<'_> {
    fn now(&self) -> DateTime<Utc> {
        self.0.now()
    }
}

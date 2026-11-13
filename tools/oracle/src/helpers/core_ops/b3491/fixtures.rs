use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::interactions::contract::records::{InteractionsLogging, Params};
use betterlite_core::api::interactions::streaming::logging::InteractionsStreamLogging;
use betterlite_core::api::interactions::streaming::records::{
    InteractionsSuccessCall, SharedApiBase,
};
use betterlite_core::api::interactions::streaming::response::{
    InteractionsAsyncResponse, InteractionsLineStream, InteractionsSyncResponse,
};
use betterlite_interactions::types::response::InteractionsStreamChunk;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_models::api_base_lookup::{
    ApiBaseContext, ApiBaseError, ApiBaseHooks, LoginFreeProvider,
};
use betterlite_models::resolution::ResolutionContext;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("the fixture stream failed")]
pub(crate) struct FixtureSourceError;

pub(crate) struct FakeAsyncResponse {
    lines: Vec<Result<String, FixtureSourceError>>,
}

impl FakeAsyncResponse {
    pub(crate) fn new(lines: &[String]) -> Self {
        Self {
            lines: lines.iter().map(|line| Ok(line.clone())).collect(),
        }
    }
}

pub(crate) struct FakeAsyncLines {
    lines: std::vec::IntoIter<Result<String, FixtureSourceError>>,
}

#[async_trait]
impl InteractionsAsyncResponse for FakeAsyncResponse {
    type Error = FixtureSourceError;
    type Lines = FakeAsyncLines;

    fn headers(&self) -> IndexMap<String, String> {
        IndexMap::new()
    }

    fn aiter_lines(self) -> Self::Lines {
        FakeAsyncLines {
            lines: self.lines.into_iter(),
        }
    }
}

#[async_trait]
impl InteractionsLineStream for FakeAsyncLines {
    type Error = FixtureSourceError;

    async fn next_line(&mut self) -> Result<Option<String>, Self::Error> {
        match self.lines.next() {
            None => Ok(None),
            Some(Ok(line)) => Ok(Some(line)),
            Some(Err(error)) => Err(error),
        }
    }
}

pub(crate) struct FakeSyncResponse {
    lines: Vec<Result<String, FixtureSourceError>>,
}

impl FakeSyncResponse {
    pub(crate) fn new(lines: &[String]) -> Self {
        Self {
            lines: lines.iter().map(|line| Ok(line.clone())).collect(),
        }
    }
}

impl InteractionsSyncResponse for FakeSyncResponse {
    type Error = FixtureSourceError;
    type Lines = std::vec::IntoIter<Result<String, FixtureSourceError>>;

    fn headers(&self) -> IndexMap<String, String> {
        IndexMap::new()
    }

    fn iter_lines(self) -> Self::Lines {
        self.lines.into_iter()
    }
}

fn fixture_moment() -> DateTime<FixedOffset> {
    match DateTime::parse_from_rfc3339("2026-05-20T00:00:00+00:00") {
        Ok(moment) => moment,
        Err(_) => DateTime::<chrono::Utc>::UNIX_EPOCH.fixed_offset(),
    }
}

pub(crate) struct FixedClock {
    now: DateTime<FixedOffset>,
}

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub(crate) struct DispatchOutput {
    pub(crate) handler: &'static str,
    pub(crate) result: InteractionsStreamChunk,
    pub(crate) cache_hit: Option<bool>,
    pub(crate) prefer_async_handlers: Option<bool>,
}

pub(crate) struct RecordingLogging {
    logging: InteractionsLogging,
    clock: FixedClock,
    records: Mutex<Vec<DispatchOutput>>,
}

impl RecordingLogging {
    pub(crate) fn new(litellm_params: Params) -> Self {
        Self {
            logging: InteractionsLogging {
                model_call_details: litellm_params,
            },
            clock: FixedClock {
                now: fixture_moment(),
            },
            records: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn records(&self) -> Vec<DispatchOutput> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(
        &self,
        handler: &'static str,
        call: InteractionsSuccessCall,
        prefer_async_handlers: Option<bool>,
    ) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(DispatchOutput {
                handler,
                result: call.response,
                cache_hit: call.cache_hit,
                prefer_async_handlers,
            });
    }
}

impl InteractionsStreamLogging for RecordingLogging {
    fn logging(&mut self) -> &mut InteractionsLogging {
        &mut self.logging
    }

    fn clock(&self) -> &dyn SuccessClock {
        &self.clock
    }

    fn dispatch_success_handlers(&self, call: InteractionsSuccessCall) {
        self.record(
            "dispatch_success_handlers",
            call,
            Some(call.prefer_async_handlers),
        );
    }

    fn run_async_success_handler(&self, call: InteractionsSuccessCall) {
        self.record("async_success_handler", call, None);
    }

    fn submit_success_handler(&self, call: InteractionsSuccessCall) {
        self.record("success_handler", call, None);
    }
}

pub(crate) struct NoHooks;

impl ApiBaseHooks for NoHooks {
    fn login_free_api_base(&self, _provider: LoginFreeProvider) -> Option<String> {
        None
    }

    fn vertex_claude_url(
        &self,
        _location: &str,
        _project: &str,
        _model: &str,
        _stream: bool,
    ) -> Result<String, ApiBaseError> {
        Err(ApiBaseError::InvalidVertexLocation {
            message: String::new(),
        })
    }
}

pub(crate) struct ApiBaseContexts<'context> {
    pub(crate) resolution: ResolutionContext<'context>,
    pub(crate) aliases: BTreeMap<String, String>,
    pub(crate) hooks: NoHooks,
}

pub(crate) fn api_resolver(contexts: &ApiBaseContexts<'_>) -> SharedApiBase<'_> {
    SharedApiBase {
        context: ApiBaseContext {
            model_alias_map: &contexts.aliases,
            hooks: &contexts.hooks,
            resolution: &contexts.resolution,
        },
    }
}

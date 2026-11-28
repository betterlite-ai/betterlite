use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_context::resource_config_selection::skills_evals::EvalsConfigTag;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_models::decisions::Json;
use betterlite_openai::evals::types::{ListRunsResponse, Run};
use betterlite_openai::openai_base_config::{Headers, Object};
use indexmap::IndexMap;

use super::error::EvalRunCallError;
use crate::api::eval_http::exchange::EvalConfig;

// the kwargs key LiteLLM's acreate_run writes and its create_run pops
pub(crate) const CREATE_FLAG: &str = "acreate_run";
// the kwargs key LiteLLM's alist_runs writes and its list_runs pops
pub(crate) const LIST_FLAG: &str = "alist_runs";

// what the two entries hand the transport: the handler future the async flag asked for, or the blocking one
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Sync,
    Async,
}

impl RunMode {
    // LiteLLM's `kwargs.pop("<operation>", False) is True`: only the boolean true selects the async twin
    #[must_use]
    pub fn of(flag: &str, kwargs: &IndexMap<String, Json>) -> Self {
        if matches!(kwargs.get(flag), Some(Json::Bool(true))) {
            Self::Async
        } else {
            Self::Sync
        }
    }
}

pub type PendingCreateRun<'call> =
    Pin<Box<dyn Future<Output = Result<Run, EvalRunCallError>> + Send + 'call>>;

pub type PendingListRuns<'call> =
    Pin<Box<dyn Future<Output = Result<ListRunsResponse, EvalRunCallError>> + Send + 'call>>;

// B5360's selector answers a tag, and the caller installs the config it built for that tag
pub trait EvalRunProviders: Send + Sync {
    fn evals_config(&self, tag: EvalsConfigTag) -> Option<EvalConfig<'_>>;
}

// everything B9371's create_run_handler is given, owned so the pending future outlives the caller
pub struct CreateRunCall<'call> {
    pub config: EvalConfig<'call>,
    pub provider: String,
    pub url: String,
    pub headers: Headers,
    pub body: Object,
    pub timeout: Duration,
    pub mode: RunMode,
}

// everything B9371's list_runs_handler is given; the handler sends no timeout, matching LiteLLM's GET
pub struct ListRunsCall<'call> {
    pub config: EvalConfig<'call>,
    pub provider: String,
    pub url: String,
    pub headers: Headers,
    pub query: Object,
    pub timeout: Duration,
    pub mode: RunMode,
}

// the provider handlers behind an injectable seam: B9371's create_run_handler and list_runs_handler in production
pub trait RunTransport: Send + Sync {
    fn create_run<'call>(
        &'call self,
        log: LogContext<'call, '_>,
        call: CreateRunCall<'call>,
    ) -> PendingCreateRun<'call>;

    fn list_runs<'call>(
        &'call self,
        log: LogContext<'call, '_>,
        call: ListRunsCall<'call>,
    ) -> PendingListRuns<'call>;
}

use std::sync::Arc;

use betterlite_core::rules::RuleEvaluator;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};

use super::CallWrapper;
use super::cache::CacheMode;
use super::error::{CallError, plain_error};
use super::fields::{field, object_field};
use super::logging::success_result;
use super::ports::BlockingBridge;
use super::post_call::{PostCallInput, Validation, run as post_call_processing};
use super::retry::max_retries_per_request_hit;
use super::state::{LoggingHandle, WrapperOptions};
use super::steps::{self, CallView};
use super::target::CallTarget;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;
use crate::call_setup::request_flags::is_streaming_request;

mod failure;

// One run of wrapper(): the request and the pieces the except block needs afterwards
pub(super) struct SyncRun<'run, E, B: BlockingBridge> {
    wrapper: &'run mut CallWrapper<E>,
    target: &'run dyn CallTarget,
    bridge: &'run B,
    call_type: String,
    request: RequestMap,
    logging: Option<LoggingHandle>,
    model: Option<String>,
    start: DateTime<FixedOffset>,
    end: Option<DateTime<FixedOffset>>,
}

impl<'run, E, B> SyncRun<'run, E, B>
where
    E: RuleEvaluator,
    B: BlockingBridge,
{
    pub(super) fn new(
        wrapper: &'run mut CallWrapper<E>,
        target: &'run dyn CallTarget,
        request: RequestMap,
        options: WrapperOptions,
        bridge: &'run B,
    ) -> Self {
        let start = wrapper.clock.now();
        Self {
            wrapper,
            target,
            bridge,
            call_type: target.name().to_owned(),
            request,
            logging: options.logging,
            model: None,
            start,
            end: None,
        }
    }

    pub(super) fn run(&mut self) -> Result<CallOutcome, CallError> {
        steps::debug_request(&self.call_type, &self.request, self.wrapper.debugging);
        match self.execute() {
            Ok(outcome) => Ok(outcome),
            Err(error) => self.handle(error),
        }
    }

    fn execute(&mut self) -> Result<CallOutcome, CallError> {
        self.prepare()?;
        if let Some(cached) = self.cached()? {
            return Ok(cached);
        }
        steps::modify_max_tokens(
            self.wrapper,
            &self.call_type,
            &mut self.request,
            self.model.as_deref(),
        )?;
        let outcome = self.send_call()?;
        self.end = Some(self.wrapper.clock.now());
        let times = steps::times(self.start, self.end, self.wrapper.clock.as_ref());
        let complete: Option<bool> = field(&self.request, "complete_response")?;
        if is_streaming_request(&self.request, &self.call_type) {
            if complete == Some(true) {
                return steps::assemble(
                    self.wrapper,
                    &self.request,
                    self.logging.as_ref(),
                    &outcome,
                );
            }
            steps::update_metadata(
                self.wrapper,
                self.logging.as_ref(),
                CallView {
                    request: &self.request,
                    model: self.model.as_deref(),
                },
                times,
                &outcome,
            );
            return Ok(outcome);
        }
        if async_result(&self.request) {
            return Ok(outcome);
        }
        let input = PostCallInput {
            call_type: &self.call_type,
            model: self.model.as_deref(),
            request: &self.request,
            outcome: &outcome,
            validation: Validation {
                enable_json_schema_validation: self.wrapper.flags.enable_json_schema_validation,
                limits: self.wrapper.flags.schema_limits,
            },
        };
        post_call_processing(&self.wrapper.rules, &input)?;
        self.store(&outcome)?;
        steps::update_metadata(
            self.wrapper,
            self.logging.as_ref(),
            CallView {
                request: &self.request,
                model: self.model.as_deref(),
            },
            times,
            &outcome,
        );
        if let Some(handle) = &self.logging {
            self.wrapper
                .logging
                .success(handle, success_result(&outcome, &self.call_type), times);
        }
        Ok(outcome)
    }

    // litellm_call_id, function_setup, credentials, the budget check and the retry limit, in that order
    fn prepare(&mut self) -> Result<(), CallError> {
        steps::insert_call_id(self.wrapper, &mut self.request);
        self.model = field(&self.request, "model")?;
        if self.logging.is_none() {
            let started =
                self.wrapper
                    .start_logging(&self.call_type, self.request.clone(), false)?;
            self.logging = Some(started.0);
            self.request = started.1;
        }
        steps::load_credentials(self.wrapper, &mut self.request)?;
        self.wrapper.budget.check()?;
        if max_retries_per_request_hit(&self.request, self.wrapper.retry.num_retries_per_request) {
            return Err(CallError::from(plain_error("Max retries per request hit!")));
        }
        Ok(())
    }

    fn cached(&mut self) -> Result<Option<CallOutcome>, CallError> {
        if !self.cache_enabled()? {
            return Ok(None);
        }
        let Some(cache) = self.wrapper.cache.as_ref().map(Arc::clone) else {
            return Ok(None);
        };
        let outcome =
            self.bridge
                .block_on(cache.lookup(&self.request, &self.call_type, CacheMode::Sync))?;
        Ok(outcome.and_then(|found| found.cached))
    }

    // (caching is unset and litellm.cache exists) or caching is true, unless cache.no-cache disables it
    fn cache_enabled(&self) -> Result<bool, CallError> {
        let caching: Option<bool> = field(&self.request, "caching")?;
        let enabled = caching.unwrap_or(self.wrapper.cache.is_some());
        if !enabled {
            return Ok(false);
        }
        let no_cache = object_field(&self.request, "cache")?
            .is_some_and(|cache| matches!(cache.get("no-cache"), Some(Json::Bool(true))));
        Ok(!no_cache)
    }

    fn send_call(&mut self) -> Result<CallOutcome, CallError> {
        let request = self.request.clone();
        let bridge = self.bridge;
        let target = self.target;
        bridge.block_on(target.call(request, self.logging.as_ref()))
    }

    fn store(&mut self, outcome: &CallOutcome) -> Result<(), CallError> {
        let Some(cache) = self.wrapper.cache.as_ref().map(Arc::clone) else {
            return Ok(());
        };
        let bridge = self.bridge;
        bridge.block_on(cache.store(&self.request, outcome, &self.call_type, CacheMode::Sync))
    }
}

// wrapper()'s opening branch: an async-flagged request skips setup, budget, cache and logging
pub(super) fn async_request_path<E, B>(
    wrapper: &CallWrapper<E>,
    target: &dyn CallTarget,
    bridge: &B,
    request: &RequestMap,
    logging: Option<&LoggingHandle>,
) -> Result<CallOutcome, CallError>
where
    E: RuleEvaluator,
    B: BlockingBridge,
{
    if max_retries_per_request_hit(request, wrapper.retry.num_retries_per_request) {
        return Err(CallError::from(plain_error("Max retries per request hit!")));
    }
    let outcome = bridge.block_on(target.call(request.clone(), logging))?;
    let complete: Option<bool> = field(request, "complete_response")?;
    if is_streaming_request(request, target.name()) && complete == Some(true) {
        return steps::assemble(wrapper, request, logging, &outcome);
    }
    Ok(outcome)
}

// the elif at L1843: a result from an async sibling call is returned before any post-call step
fn async_result(request: &RequestMap) -> bool {
    [
        "acompletion",
        "aembedding",
        "aimg_generation",
        "atranscription",
        "aspeech",
    ]
    .iter()
    .any(|flag| matches!(request.get(*flag), Some(Json::Bool(true))))
}

use std::sync::Arc;

use betterlite_core::rules::RuleEvaluator;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use serde_json::Number;

use super::CallWrapper;
use super::cache::CacheMode;
use super::error::{CallError, retry_limit_error};
use super::fields::{bool_field, int_field, object_field, text_field};
use super::logging::success_result;
use super::max_tokens::MODIFIED_CALL_TYPES;
use super::ports::{BlockingBridge, CallTimes};
use super::post_call::{self, PostCallInput, Validation};
use super::retry::{
    context_window_model, is_api_error, is_context_window, is_router_call,
    max_retries_per_request_hit, sync_num_retries,
};
use super::state::{LoggingHandle, WrapperOptions};
use super::target::CallTarget;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;
use crate::call_setup::credentials::load_credentials_from_list;
use crate::call_setup::request_flags::{is_async_request, is_streaming_request};
use crate::call_setup::text::debug_text;

// One run of wrapper(): the request and the pieces the except block needs afterwards
struct SyncRun<'run, E> {
    wrapper: &'run mut CallWrapper<E>,
    target: &'run dyn CallTarget,
    bridge: &'run dyn BlockingBridge,
    call_type: String,
    request: RequestMap,
    logging: Option<LoggingHandle>,
    model: Option<String>,
    start: DateTime<FixedOffset>,
    end: Option<DateTime<FixedOffset>>,
}

pub(super) fn run<E>(
    wrapper: &mut CallWrapper<E>,
    target: &dyn CallTarget,
    request: RequestMap,
    options: WrapperOptions,
    bridge: &dyn BlockingBridge,
) -> Result<CallOutcome, CallError>
where
    E: RuleEvaluator,
{
    let start = wrapper.clock.now();
    let mut run = SyncRun {
        wrapper,
        target,
        bridge,
        call_type: target.name().to_owned(),
        request,
        logging: options.logging,
        model: None,
        start,
        end: None,
    };
    if is_async_request(&run.request, false) {
        return run.async_request_path();
    }
    run.print_request();
    match run.execute() {
        Ok(outcome) => Ok(outcome),
        Err(error) => run.failure(error),
    }
}

impl<E> SyncRun<'_, E>
where
    E: RuleEvaluator,
{
    // the short path: an async-flagged request skips setup, budget, cache and logging
    fn async_request_path(&mut self) -> Result<CallOutcome, CallError> {
        if max_retries_per_request_hit(&self.request, self.wrapper.retry.num_retries_per_request) {
            return Err(retry_limit_error());
        }
        let outcome = self.send_call()?;
        if is_streaming_request(&self.request, &self.call_type) && complete_response(&self.request)?
        {
            return self.assemble(&outcome);
        }
        Ok(outcome)
    }

    fn print_request(&self) {
        match debug_text(&self.call_type, &self.request, self.wrapper.debugging) {
            Ok(Some(text)) => tracing::debug!("{text}"),
            Ok(None) => {}
            Err(error) => tracing::debug!(%error, "the debug request line could not be rendered"),
        }
    }

    fn execute(&mut self) -> Result<CallOutcome, CallError> {
        self.prepare()?;
        if let Some(cached) = self.cached()? {
            return Ok(cached);
        }
        self.modify_max_tokens()?;
        let outcome = self.send_call()?;
        self.end = Some(self.wrapper.clock.now());
        if is_streaming_request(&self.request, &self.call_type) {
            if complete_response(&self.request)? {
                return self.assemble(&outcome);
            }
            self.update_metadata(&outcome);
            return Ok(outcome);
        }
        if async_result(&self.request) {
            return Ok(outcome);
        }
        self.post_call(&outcome)?;
        self.store(&outcome)?;
        self.update_metadata(&outcome);
        self.log_success(&outcome);
        Ok(outcome)
    }

    // litellm_call_id, function_setup, credentials, the budget check and the retry limit, in that order
    fn prepare(&mut self) -> Result<(), CallError> {
        if !self.request.contains_key("litellm_call_id") {
            let call_id = self.wrapper.ids.next_call_id();
            self.request
                .insert("litellm_call_id".to_owned(), Json::Text(call_id));
        }
        self.model = text_field(&self.request, "model")?;
        if self.logging.is_none() {
            let started =
                self.wrapper
                    .start_logging(&self.call_type, self.request.clone(), false)?;
            self.logging = Some(started.0);
            self.request = started.1;
        }
        let warning =
            load_credentials_from_list(&mut self.request, self.wrapper.credentials.as_ref())?;
        if let Some(warning) = warning {
            tracing::debug!(
                credential = %warning.credential_name,
                "no credential matches the requested name"
            );
        }
        self.wrapper.budget.check()?;
        if max_retries_per_request_hit(&self.request, self.wrapper.retry.num_retries_per_request) {
            return Err(retry_limit_error());
        }
        Ok(())
    }

    fn cached(&mut self) -> Result<Option<CallOutcome>, CallError> {
        if !self.cache_enabled()? {
            return Ok(None);
        }
        let cache = Arc::clone(&self.wrapper.cache);
        let outcome =
            self.bridge
                .block_on(cache.lookup(&self.request, &self.call_type, CacheMode::Sync))?;
        Ok(outcome.cached)
    }

    // (caching is unset and litellm.cache exists) or caching is true, unless cache.no-cache disables it
    fn cache_enabled(&self) -> Result<bool, CallError> {
        let enabled = bool_field(&self.request, "caching")?
            .unwrap_or_else(|| self.wrapper.cache.configured());
        if !enabled {
            return Ok(false);
        }
        let no_cache = object_field(&self.request, "cache")?
            .is_some_and(|cache| matches!(cache.get("no-cache"), Some(Json::Bool(true))));
        Ok(!no_cache)
    }

    fn modify_max_tokens(&mut self) -> Result<(), CallError> {
        let Some(user_max_tokens) = int_field(&self.request, "max_tokens")? else {
            return Ok(());
        };
        let Some(model) = self.model.as_deref() else {
            return Ok(());
        };
        if !self.wrapper.flags.modify_params
            || !MODIFIED_CALL_TYPES.contains(&self.call_type.as_str())
        {
            return Ok(());
        }
        let hf_model_name = text_field(&self.request, "hf_model_name")?;
        let messages = self.request.get("messages");
        let modified = self.wrapper.max_tokens.modified_max_tokens(
            model,
            hf_model_name.as_deref(),
            messages,
            user_max_tokens,
        );
        let value = modified.map_or(Json::Null, |tokens| Json::Number(Number::from(tokens)));
        self.request.insert("max_tokens".to_owned(), value);
        Ok(())
    }

    fn send_call(&mut self) -> Result<CallOutcome, CallError> {
        let request = self.request.clone();
        self.bridge
            .block_on(self.target.call(request, self.logging.as_ref()))
    }

    fn assemble(&self, outcome: &CallOutcome) -> Result<CallOutcome, CallError> {
        let messages = self.request.get("messages");
        let assembled =
            self.wrapper
                .stream_assembly
                .assemble(outcome, messages, self.logging.as_ref())?;
        Ok(CallOutcome::Response(assembled.unwrap_or(Json::Null)))
    }

    fn post_call(&self, outcome: &CallOutcome) -> Result<(), CallError> {
        let input = PostCallInput {
            call_type: &self.call_type,
            model: self.model.as_deref(),
            request: &self.request,
            outcome,
            validation: Validation {
                enable_json_schema_validation: self.wrapper.flags.enable_json_schema_validation,
                limits: self.wrapper.flags.schema_limits,
            },
        };
        post_call::run(&self.wrapper.rules, &input)
    }

    fn store(&mut self, outcome: &CallOutcome) -> Result<(), CallError> {
        let cache = Arc::clone(&self.wrapper.cache);
        self.bridge
            .block_on(cache.store(&self.request, outcome, &self.call_type, CacheMode::Sync))
    }

    fn update_metadata(&self, outcome: &CallOutcome) {
        let Some(handle) = &self.logging else {
            return;
        };
        self.wrapper.metadata.update(
            outcome,
            handle,
            self.model.as_deref(),
            &self.request,
            self.times(),
        );
    }

    fn log_success(&self, outcome: &CallOutcome) {
        let Some(handle) = &self.logging else {
            return;
        };
        let result = success_result(outcome, &self.call_type);
        self.wrapper.logging.success(handle, result, self.times());
    }

    fn times(&self) -> CallTimes {
        let end = self.end.unwrap_or_else(|| self.wrapper.clock.now());
        CallTimes {
            start: self.start,
            end,
        }
    }

    // the except block: completion and responses retries, then failure logging and the original error
    fn failure(&mut self, error: CallError) -> Result<CallOutcome, CallError> {
        let retried = match self.call_type.as_str() {
            "completion" => self.retry_completion(&error)?,
            "responses" => self.retry_entry(&error)?,
            _ => None,
        };
        if let Some(outcome) = retried {
            return Ok(outcome);
        }
        self.log_failure(&error);
        Err(error)
    }

    fn retry_completion(&mut self, error: &CallError) -> Result<Option<CallOutcome>, CallError> {
        if let Some(outcome) = self.retry_entry(error)? {
            return Ok(Some(outcome));
        }
        self.context_fallback(error)
    }

    // the count is the request's, then the default; a retry policy replaces it and is reset first
    fn retry_entry(&mut self, error: &CallError) -> Result<Option<CallOutcome>, CallError> {
        let count = sync_num_retries(&mut self.request, self.wrapper.retry.num_retries, error)?;
        let router_call = is_router_call(&self.request)?;
        let Some(count) = count.filter(|value| *value != 0) else {
            return Ok(None);
        };
        if router_call || !is_api_error(&error.class) {
            return Ok(None);
        }
        self.request
            .insert("num_retries".to_owned(), Json::Number(Number::from(count)));
        let request = self.request.clone();
        let outcome = self
            .bridge
            .block_on(self.target.retry(request, self.logging.as_ref()))?;
        Ok(Some(outcome))
    }

    fn context_fallback(&mut self, error: &CallError) -> Result<Option<CallOutcome>, CallError> {
        if !is_context_window(&error.class) || is_router_call(&self.request)? {
            return Ok(None);
        }
        let mapped = context_window_model(&self.request, self.model.as_deref())?;
        let Some(model) = mapped else {
            return Ok(None);
        };
        self.request.insert("model".to_owned(), Json::Text(model));
        let request = self.request.clone();
        let outcome = self
            .bridge
            .block_on(self.target.call(request, self.logging.as_ref()))?;
        Ok(Some(outcome))
    }

    fn log_failure(&self, error: &CallError) {
        let Some(handle) = &self.logging else {
            return;
        };
        self.wrapper
            .logging
            .failure_sync(handle, error, self.times());
    }
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

fn complete_response(request: &RequestMap) -> Result<bool, CallError> {
    Ok(bool_field(request, "complete_response")?.is_some_and(|flag| flag))
}

use std::sync::PoisonError;

use betterlite_core::rules::RuleEvaluator;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};

use super::CallWrapper;
use super::cache::CacheOutcome;
use super::error::CallError;
use super::fields::field;
use super::state::{LoggingHandle, WrapperOptions};
use super::steps;
use super::target::CallTarget;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;
use crate::call_setup::hooks::{failure_deployment_hook, pre_call_deployment_hook};
use crate::call_setup::request_flags::is_streaming_request;

mod cache;
mod failure;
mod success;

// One run of wrapper_async(): the request, the kwargs the deployment saw, and the except block's state
pub(super) struct AsyncRun<'run, E> {
    wrapper: &'run mut CallWrapper<E>,
    target: &'run dyn CallTarget,
    call_type: String,
    request: RequestMap,
    call_kwargs: RequestMap,
    logging: Option<LoggingHandle>,
    model: Option<String>,
    start: DateTime<FixedOffset>,
    end: Option<DateTime<FixedOffset>>,
    deployment_end: Option<DateTime<FixedOffset>>,
    internal_call: bool,
    is_completion_with_fallbacks: bool,
    cache: Option<CacheOutcome>,
    // the local `result` the finally block inspects: only a chat stream skips the correlation restore
    chat_stream_result: bool,
}

impl<'run, E> AsyncRun<'run, E>
where
    E: RuleEvaluator,
{
    pub(super) fn new(
        wrapper: &'run mut CallWrapper<E>,
        target: &'run dyn CallTarget,
        request: RequestMap,
        options: WrapperOptions,
    ) -> Self {
        let start = wrapper.clock.now();
        let call_type = target.name().to_owned();
        let is_completion_with_fallbacks =
            !matches!(request.get("fallbacks"), None | Some(Json::Null));
        Self {
            wrapper,
            target,
            call_type,
            call_kwargs: request.clone(),
            request,
            logging: options.logging,
            model: None,
            start,
            end: None,
            deployment_end: None,
            internal_call: options.internal_call,
            is_completion_with_fallbacks,
            cache: None,
            chat_stream_result: false,
        }
    }

    pub(super) async fn run(&mut self) -> Result<CallOutcome, CallError> {
        steps::debug_request(&self.call_type, &self.request, self.wrapper.debugging);
        let outcome = match self.execute().await {
            Ok(outcome) => Ok(outcome),
            Err(error) => failure::handle(self, error).await,
        };
        if !self.chat_stream_result {
            failure::restore_correlation(self);
        }
        outcome
    }

    async fn execute(&mut self) -> Result<CallOutcome, CallError> {
        steps::insert_call_id(self.wrapper, &mut self.request);
        self.model = field(&self.request, "model")?;
        self.request.shift_remove("_is_litellm_internal_call");
        if self.logging.is_none() {
            let (handle, request) =
                self.wrapper
                    .start_logging(&self.call_type, self.request.clone(), true)?;
            self.logging = Some(handle);
            self.request = request;
        }
        let hooked = pre_call_deployment_hook(
            &self.wrapper.setup.callbacks,
            &self.request,
            &self.call_type,
        )
        .await?;
        self.request = hooked;
        self.sync_stream()?;
        steps::load_credentials(self.wrapper, &mut self.request)?;
        self.wrapper.budget.check()?;
        if let Some(cached) = cache::lookup(self).await? {
            return Ok(cached);
        }
        cache::apply_preset_cache_key(self);
        steps::modify_max_tokens(
            self.wrapper,
            &self.call_type,
            &mut self.request,
            self.model.as_deref(),
        )?;
        self.call_kwargs = self.request.clone();
        if let Some(input) = self
            .cache
            .as_ref()
            .and_then(|found| found.uncached_input.clone())
        {
            self.call_kwargs.insert("input".to_owned(), input);
        }
        let outcome = self.send_call().await?;
        self.end = Some(self.wrapper.clock.now());
        let streaming_requested = is_streaming_request(&self.request, &self.call_type);
        if streaming_requested || outcome.is_converted_stream() {
            return success::stream_result(self, outcome, streaming_requested).await;
        }
        if matches!(
            self.call_type.as_str(),
            "arealtime" | "aresponses_websocket"
        ) {
            return Ok(outcome);
        }
        success::post_call(self, &outcome)?;
        success::finish(self, outcome).await
    }

    // the deployment hooks may convert the request, so the logging object follows the hook's stream value
    fn sync_stream(&self) -> Result<(), CallError> {
        let stream: Option<bool> = field(&self.request, "stream")?;
        let Some(stream) = stream else {
            return Ok(());
        };
        let Some(handle) = &self.logging else {
            return Ok(());
        };
        let mut state = handle.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.stream != Some(stream) {
            state.stream = Some(stream);
        }
        Ok(())
    }

    // the call sits in its own error handler so the failure hook still runs; its error replaces nothing
    async fn send_call(&mut self) -> Result<CallOutcome, CallError> {
        let request = self.call_kwargs.clone();
        let target = self.target;
        let logging = self.logging.clone();
        match target.call(request, logging.as_ref()).await {
            Ok(outcome) => {
                self.chat_stream_result = outcome.is_streaming_response();
                Ok(outcome)
            }
            Err(error) => {
                self.deployment_end = Some(self.wrapper.clock.now());
                failure_deployment_hook(
                    &self.wrapper.setup.callbacks,
                    &self.call_kwargs,
                    &error.class,
                    &self.call_type,
                )
                .await;
                Err(error)
            }
        }
    }
}

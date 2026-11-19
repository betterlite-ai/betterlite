use betterlite_core::rules::RuleEvaluator;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::SyncRun;
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::error::CallError;
use crate::call_wrapper::ports::BlockingBridge;
use crate::call_wrapper::retry::{
    RetryMode, context_window_model, is_api_error, is_router_call, num_retries,
};
use crate::call_wrapper::steps;

impl<E, B> SyncRun<'_, E, B>
where
    E: RuleEvaluator,
    B: BlockingBridge,
{
    // The except block: a completion retries or falls back, responses only retries, then failure logging and the error
    pub(super) fn handle(&mut self, error: CallError) -> Result<CallOutcome, CallError> {
        let call_type = self.call_type.clone();
        let retried = match call_type.as_str() {
            "completion" => self.retry_previous(&error, true)?,
            "responses" => self.retry_previous(&error, false)?,
            _ => None,
        };
        if let Some(outcome) = retried {
            return Ok(outcome);
        }
        self.log_failure(&error);
        Err(error)
    }

    // A context window error only falls back when the retry branch above wasn't entered
    fn retry_previous(
        &mut self,
        error: &CallError,
        context_fallback_allowed: bool,
    ) -> Result<Option<CallOutcome>, CallError> {
        let count = self.resolve_count(error)?;
        let router = is_router_call(&self.request)?;
        if let Some(retries) = count.filter(|value| *value != 0) {
            if router || !is_api_error(&error.class) {
                return Ok(None);
            }
            return self.retry_call(retries);
        }
        if router || !context_fallback_allowed {
            return Ok(None);
        }
        self.context_fallback(error)
    }

    // the request's count, then litellm.num_retries, then a retry policy; the default is cleared so a retry can't loop
    fn resolve_count(&mut self, error: &CallError) -> Result<Option<i64>, CallError> {
        let count = num_retries(
            &mut self.request,
            self.wrapper.retry.num_retries,
            error,
            RetryMode::Sync,
        )?;
        self.wrapper.retry.num_retries = None;
        Ok(count)
    }

    fn retry_call(&mut self, count: i64) -> Result<Option<CallOutcome>, CallError> {
        self.request
            .insert("num_retries".to_owned(), Json::Number(Number::from(count)));
        let request = self.request.clone();
        let bridge = self.bridge;
        let target = self.target;
        let outcome = bridge.block_on(target.retry(request, self.logging.as_ref()))?;
        Ok(Some(outcome))
    }

    fn context_fallback(&mut self, error: &CallError) -> Result<Option<CallOutcome>, CallError> {
        if !matches!(&error.class, ErrorClass::ContextWindowExceeded(_))
            || is_router_call(&self.request)?
        {
            return Ok(None);
        }
        let Some(model) = context_window_model(&self.request, self.model.as_deref())? else {
            return Ok(None);
        };
        self.request.insert("model".to_owned(), Json::Text(model));
        let request = self.request.clone();
        let bridge = self.bridge;
        let target = self.target;
        let outcome = bridge.block_on(target.call(request, self.logging.as_ref()))?;
        Ok(Some(outcome))
    }

    fn log_failure(&self, error: &CallError) {
        let Some(handle) = &self.logging else {
            return;
        };
        let times = steps::times(self.start, self.end, self.wrapper.clock.as_ref());
        self.wrapper.logging.failure_sync(handle, error, times);
    }
}

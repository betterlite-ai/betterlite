use std::fmt::Display;

use crate::generic_guardrail_contracts::UnreachableFallback;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::straiker::delivery::block::block_failure;
use crate::straiker::delivery::retry::RetryDelay;
use crate::straiker::delivery::transport::WebhookPost;
use crate::straiker::delivery::{ApplyRequest, StraikerDelivery, phase_name};

impl<Post, Delay> StraikerDelivery<Post, Delay>
where
    Post: WebhookPost,
    Delay: RetryDelay,
{
    pub(super) fn block(
        &self,
        request: &ApplyRequest<'_>,
        message: &str,
        blocked_content: bool,
    ) -> GuardrailFailure {
        block_failure(
            &self.settings.guardrail_name,
            request.request_data,
            request.input_type,
            message,
            blocked_content,
        )
    }

    // LiteLLM's fallback: let the call through when the endpoint is unreachable under fail_open or when fail_on_error is off, block otherwise.
    pub(super) fn fail(
        &self,
        request: &ApplyRequest<'_>,
        error: &dyn Display,
        is_unreachable: bool,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let fail_open = (is_unreachable
            && self.settings.unreachable_fallback == UnreachableFallback::FailOpen)
            || !self.settings.fail_on_error;
        tracing::error!(
            event = "straiker.error",
            input_type = phase_name(request.input_type),
            error = %error,
            fail_open,
            "straiker.error"
        );
        if fail_open {
            return Ok(request.inputs.clone());
        }
        let message = format!("Straiker detection unavailable: {error}");
        Err(self.block(request, &message, false))
    }
}

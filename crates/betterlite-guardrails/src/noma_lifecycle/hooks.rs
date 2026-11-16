use std::collections::BTreeSet;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::error::NomaLifecycleError;
use super::{MCP_CALL_TYPE, NomaGuardrail, RequestData};
use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::noma_scan_policy::ScanCallContext;

pub struct RequestHookInput<'input> {
    pub user_auth: &'input RequestData,
    pub data: &'input mut RequestData,
    pub call_type: &'input str,
    pub suppressed: &'input BTreeSet<String>,
}

pub struct PostCallHookInput<'input> {
    pub user_auth: &'input RequestData,
    pub data: &'input mut RequestData,
    pub response: &'input mut GuardrailValue,
    pub suppressed: &'input BTreeSet<String>,
}

pub(super) enum ScanSubject<'subject> {
    UserMessage,
    LlmResponse(&'subject mut GuardrailValue),
}

// async_pre_call_hook and async_moderation_hook differ only in their event type and their log text
#[derive(Debug, Clone, Copy)]
enum RequestHook {
    PreCall,
    Moderation,
}

impl RequestHook {
    const fn scan_event(self) -> GuardrailEventHooks {
        match self {
            Self::PreCall => GuardrailEventHooks::PreCall,
            Self::Moderation => GuardrailEventHooks::DuringCall,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::PreCall => "pre-call",
            Self::Moderation => "moderation",
        }
    }

    // an MCP tool call is gated as pre_mcp_call in both hooks, though the scan keeps the hook's own event
    fn gate_event(self, call_type: &str) -> GuardrailEventHooks {
        if call_type == MCP_CALL_TYPE {
            return GuardrailEventHooks::PreMcpCall;
        }
        self.scan_event()
    }
}

impl NomaGuardrail {
    // LiteLLM's _process_user_message_check and _process_llm_response_check, behind _check_user_message and _check_llm_response
    pub(super) async fn scan(
        &self,
        request_data: &mut RequestData,
        user_auth: &RequestData,
        subject: ScanSubject<'_>,
        event_type: Option<GuardrailEventHooks>,
    ) -> Result<(), NomaLifecycleError> {
        let extra_data = self.base.dynamic_request_body_params(request_data)?;
        let call = ScanCallContext {
            user_auth,
            extra_data: &extra_data,
            event_type,
        };
        match subject {
            ScanSubject::UserMessage => {
                self.policy
                    .process_user_message_check(request_data, call)
                    .await?;
            }
            ScanSubject::LlmResponse(response) => {
                self.policy
                    .process_llm_response_check(request_data, response, call)
                    .await?;
            }
        }
        Ok(())
    }

    async fn run_request_hook(
        &self,
        hook: RequestHook,
        input: RequestHookInput<'_>,
    ) -> Result<(), NomaLifecycleError> {
        let RequestHookInput {
            user_auth,
            data,
            call_type,
            suppressed,
        } = input;
        let gate_event = hook.gate_event(call_type);
        if !self
            .base
            .should_run_guardrail(data, gate_event, suppressed)?
        {
            return Ok(());
        }
        if self.policy.settings.monitor_mode {
            self.spawn_user_check(data, user_auth);
            return Ok(());
        }
        let event_type = hook.scan_event();
        if let Err(error) = self
            .scan(data, user_auth, ScanSubject::UserMessage, Some(event_type))
            .await
        {
            return self.handle_failure(error, data, event_type, hook.name());
        }
        Ok(())
    }

    pub async fn async_pre_call_hook(
        &self,
        input: RequestHookInput<'_>,
    ) -> Result<(), NomaLifecycleError> {
        tracing::debug!("Running Noma pre-call hook");
        self.run_request_hook(RequestHook::PreCall, input).await
    }

    pub async fn async_moderation_hook(
        &self,
        input: RequestHookInput<'_>,
    ) -> Result<(), NomaLifecycleError> {
        tracing::debug!("Running Noma moderation hook");
        self.run_request_hook(RequestHook::Moderation, input).await
    }

    // The response is scanned in place, so an anonymized answer is what the caller keeps
    pub async fn async_post_call_success_hook(
        &self,
        input: PostCallHookInput<'_>,
    ) -> Result<(), NomaLifecycleError> {
        let PostCallHookInput {
            user_auth,
            data,
            response,
            suppressed,
        } = input;
        let event_type = GuardrailEventHooks::PostCall;
        if !self
            .base
            .should_run_guardrail(data, event_type, suppressed)?
        {
            return Ok(());
        }
        if self.policy.settings.monitor_mode {
            self.spawn_response_check(data, response, user_auth);
            return Ok(());
        }
        let subject = ScanSubject::LlmResponse(response);
        if let Err(error) = self.scan(data, user_auth, subject, Some(event_type)).await {
            return self.handle_failure(error, data, event_type, "post-call");
        }
        Ok(())
    }
}

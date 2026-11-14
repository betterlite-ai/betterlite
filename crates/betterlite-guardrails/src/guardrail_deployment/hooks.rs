use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_core::routes::CallType;
use thiserror::Error;

use super::hook_target::{DeploymentHookTarget, PreCallOutcome, stamped_key_fields};
use super::markers::PreCallToken;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_selection::error::GuardrailSelectionError;
use crate::guardrail_selection::gate::GuardrailSelection as _;

type Data = BTreeMap<String, GuardrailValue>;

// The failure is boxed to keep the variants close in size
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DeploymentHookError {
    #[error(transparent)]
    Selection(#[from] GuardrailSelectionError),
    #[error(transparent)]
    Hook(Box<GuardrailFailure>),
}

impl From<GuardrailFailure> for DeploymentHookError {
    fn from(failure: GuardrailFailure) -> Self {
        Self::Hook(Box::new(failure))
    }
}

// What the caller knows about this request besides its body: the process token and the names an auto router turned off
#[derive(Debug, Clone)]
pub struct PreCallContext<'ctx> {
    pub token: &'ctx PreCallToken,
    pub suppressed: &'ctx BTreeSet<String>,
}

impl GuardrailBase {
    // LiteLLM's async_pre_call_deployment_hook; kwargs comes back untouched unless every step passes
    pub async fn pre_call_deployment_hook<H: DeploymentHookTarget>(
        &self,
        hooks: &H,
        context: &PreCallContext<'_>,
        mut kwargs: Data,
        call_type: Option<CallType>,
    ) -> Result<Data, DeploymentHookError> {
        if !matches!(kwargs.get("guardrails"), Some(GuardrailValue::Array(_)))
            || self.pre_call_hook_already_ran(context.token, &kwargs)
        {
            return Ok(kwargs);
        }
        if !self.should_run_guardrail(&kwargs, GuardrailEventHooks::PreCall, context.suppressed)? {
            return Ok(kwargs);
        }
        let Some(call_type @ (CallType::Completion | CallType::Acompletion)) = call_type else {
            return Ok(kwargs);
        };
        let key = stamped_key_fields(&kwargs);
        let outcome = hooks
            .pre_call(self.hook_target(), &key, &mut kwargs, call_type)
            .await?;
        if let PreCallOutcome::Data(mut result) = outcome
            && let Some(messages) = result.remove("messages")
            && messages != GuardrailValue::Null
        {
            kwargs.insert("messages".to_owned(), messages);
        }
        Ok(kwargs)
    }

    // LiteLLM's async_post_call_success_deployment_hook, without success for the name limit; LiteLLM's call_type argument is never read
    pub async fn post_call_deployment_hook<H: DeploymentHookTarget>(
        &self,
        hooks: &H,
        suppressed: &BTreeSet<String>,
        request_data: &mut Data,
        response: GuardrailValue,
    ) -> Result<Option<GuardrailValue>, DeploymentHookError> {
        if !matches!(
            request_data.get("guardrails"),
            Some(GuardrailValue::Array(_))
        ) {
            return Ok(None);
        }
        if !self.should_run_guardrail(request_data, GuardrailEventHooks::PostCall, suppressed)? {
            return Ok(None);
        }
        let key = stamped_key_fields(request_data);
        Ok(hooks
            .post_call_success(self.hook_target(), &key, request_data, response)
            .await?)
    }
}

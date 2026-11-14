use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_core::routes::CallType;
use thiserror::Error;

use super::hook_target::{
    DeploymentHookTarget, HookTarget, PreCallOutcome, metadata_bucket, stamped_key_fields,
};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;

type Data = BTreeMap<String, GuardrailValue>;

// The request keys that carry the content being scanned, which the scan replaces with its own messages
const CONTENT_KEYS: [&str; 9] = [
    "messages",
    "input",
    "prompt",
    "system",
    "instructions",
    "tools",
    "functions",
    "function_call",
    "tool_choice",
];

// The caller turns Rejected into a 400 with detail {"error": message} and adds the guardrail name and event hook to it
#[derive(Debug, Clone, PartialEq, Error)]
pub enum MessagesHookError {
    #[error("{message}")]
    Rejected {
        message: String,
        guardrail_name: Option<String>,
        event_hook: Option<LitellmParamsMode>,
    },
    #[error("{failure}")]
    Failed {
        failure: GuardrailFailure,
        guardrail_name: Option<String>,
        event_hook: Option<LitellmParamsMode>,
    },
}

impl GuardrailBase {
    fn rejected(&self, message: String) -> MessagesHookError {
        MessagesHookError::Rejected {
            message,
            guardrail_name: self.settings.guardrail_name.clone(),
            event_hook: self.settings.event_hook.clone(),
        }
    }

    fn failed(&self, failure: GuardrailFailure) -> MessagesHookError {
        MessagesHookError::Failed {
            failure,
            guardrail_name: self.settings.guardrail_name.clone(),
            event_hook: self.settings.event_hook.clone(),
        }
    }

    // An error from the hook: a reroute cannot happen after retrieval, so it blocks; any other failure is raised as it is
    fn hook_error(&self, failure: GuardrailFailure) -> MessagesHookError {
        if let GuardrailFailure::SensitiveDataRoute(route) = &failure {
            let name = route
                .guardrail_name
                .as_deref()
                .or(self.settings.guardrail_name.as_deref())
                .unwrap_or("None");
            return self.rejected(format!(
                "{name} asked to reroute the request to {} over retrieved content; a request cannot be rerouted after retrieval, so it was blocked",
                route.route_to_model
            ));
        }
        self.failed(failure)
    }

    // LiteLLM's async_pre_call_hook_on_messages: scans retrieved content with the pre-call hook
    pub async fn pre_call_hook_on_messages<H: DeploymentHookTarget>(
        &self,
        hooks: &H,
        request_data: &Data,
        messages: &[GuardrailValue],
    ) -> Result<Vec<GuardrailValue>, MessagesHookError> {
        let target = self.hook_target();
        let mut scan_request: Data = request_data
            .iter()
            .filter(|(name, _value)| !CONTENT_KEYS.contains(&name.as_str()))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        scan_request.insert(
            "messages".to_owned(),
            GuardrailValue::Array(messages.to_vec()),
        );
        if target == HookTarget::Unified
            && let Some(bucket @ GuardrailValue::Object(_)) = metadata_bucket(request_data)
        {
            scan_request.insert("litellm_metadata".to_owned(), bucket.clone());
        }
        let key = stamped_key_fields(&scan_request);
        let outcome = hooks
            .pre_call(target, &key, &mut scan_request, CallType::Acompletion)
            .await
            .map_err(|failure| self.hook_error(failure))?;
        match outcome {
            PreCallOutcome::Unchanged => Ok(messages.to_vec()),
            PreCallOutcome::Data(mut result) => match result.remove("messages") {
                Some(GuardrailValue::Array(scanned)) => Ok(scanned),
                _ => Ok(messages.to_vec()),
            },
            PreCallOutcome::Text(message) => Err(self.rejected(message)),
            PreCallOutcome::Failure(failure) => Err(self.failed(*failure)),
        }
    }
}

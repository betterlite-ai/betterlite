pub mod error;
pub mod metadata;
pub mod names;
pub mod refusals;

mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use error::ToolPolicyError;
use metadata::permission_ids_from_data;
use names::{request_tool_names, response_tool_names};
use refusals::{blocked_failure, trust_chain_failure};

pub const DEFAULT_GUARDRAIL_NAME: &str = "tool_policy";
pub const POLICY_BLOCKED: &str = "blocked";
pub const POLICY_TRUSTED: &str = "trusted";
pub const POLICY_UNTRUSTED: &str = "untrusted";

// ToolPolicyGuardrail defines apply_guardrail, so LiteLLM accepts logging_only for it
const TOOL_POLICY_SWITCHES: GuardrailSwitches = GuardrailSwitches {
    use_native_during_call_hook: false,
    use_native_lifecycle_hooks: false,
    records_own_information: false,
    uses_apply_guardrail_interface: true,
    deny_text_on_error: false,
};

pub trait ToolPolicyLookup: Sync {
    fn is_initialized(&self) -> bool;
    fn effective_policies(
        &self,
        tool_names: &[String],
        key_permission_id: Option<&str>,
        team_permission_id: Option<&str>,
    ) -> BTreeMap<String, String>;
    fn output_policy(&self, tool_name: &str) -> String;
}

pub trait RequestToolNames: Sync {
    fn names(&self, route: &str, request_data: &BTreeMap<String, GuardrailValue>) -> Vec<String>;
}

pub struct ToolPolicyGuardrail {
    pub base: GuardrailBase,
    pub policies: Arc<dyn ToolPolicyLookup + Send + Sync>,
    pub names: Arc<dyn RequestToolNames + Send + Sync>,
}

pub fn initialize_tool_policy(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
    strict_modes: bool,
    policies: Arc<dyn ToolPolicyLookup + Send + Sync>,
    names: Arc<dyn RequestToolNames + Send + Sync>,
) -> Result<ToolPolicyGuardrail, ToolPolicyError> {
    let name = guardrail_name.unwrap_or(DEFAULT_GUARDRAIL_NAME);
    // LiteLLM's initialize_guardrail passes a name, the mode and default_on, and nothing else
    ToolPolicyGuardrail::new(
        GuardrailSettings {
            guardrail_name: Some(name.to_owned()),
            supported_event_hooks: None,
            event_hook: Some(params.mode.clone()),
            default_on: params.base.fields.default_on.unwrap_or_default(),
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: None,
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout: None,
            switches: TOOL_POLICY_SWITCHES,
        },
        strict_modes,
        policies,
        names,
    )
}

fn trusted_tools(policy_map: &BTreeMap<String, String>, tool_names: &[String]) -> Vec<String> {
    tool_names
        .iter()
        .filter(|name| {
            policy_map
                .get(name.as_str())
                .is_some_and(|policy| policy.as_str() == POLICY_TRUSTED)
        })
        .cloned()
        .collect()
}

impl ToolPolicyGuardrail {
    pub fn new(
        mut settings: GuardrailSettings,
        strict_modes: bool,
        policies: Arc<dyn ToolPolicyLookup + Send + Sync>,
        names: Arc<dyn RequestToolNames + Send + Sync>,
    ) -> Result<Self, ToolPolicyError> {
        if settings.supported_event_hooks.is_none() {
            settings.supported_event_hooks = Some(vec![
                GuardrailEventHooks::PreCall,
                GuardrailEventHooks::PostCall,
                GuardrailEventHooks::DuringCall,
            ]);
        }
        let base = GuardrailBase::new(settings, strict_modes)?;
        Ok(Self {
            base,
            policies,
            names,
        })
    }

    fn enforce(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let tool_names = match input_type {
            GuardrailInputType::Request => {
                request_tool_names(inputs.tools.as_deref(), self.names.as_ref(), request_data)
            }
            GuardrailInputType::Response => response_tool_names(inputs.tool_calls.as_deref()),
        };
        if tool_names.is_empty() {
            return Ok(inputs);
        }
        let (key_id, team_id) = permission_ids_from_data(request_data);
        if !self.policies.is_initialized() {
            return Ok(inputs);
        }
        let policy_map =
            self.policies
                .effective_policies(&tool_names, key_id.as_deref(), team_id.as_deref());
        if let Some(failure) = blocked_failure(&policy_map, &tool_names) {
            return Err(failure);
        }
        if input_type == GuardrailInputType::Response {
            let trusted = trusted_tools(&policy_map, &tool_names);
            if let Some(failure) =
                trust_chain_failure(&trusted, request_data, self.policies.as_ref())
            {
                return Err(failure);
            }
        }
        Ok(inputs)
    }
}

impl GuardrailContract for ToolPolicyGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        let outcome = self.enforce(inputs, request_data, input_type);
        async move { outcome }
    }
}

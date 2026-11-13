use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::failure::GuardrailFailure;
use super::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::generic_guardrail_contracts::GuardrailInputType;

pub trait GuardrailContract: Sync {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        _request_data: &mut BTreeMap<String, GuardrailValue>,
        _input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        async move { Ok(inputs) }
    }

    fn get_config_model(&self) -> Option<GuardrailValue> {
        None
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        None
    }

    fn supports_tool_result_scan(&self) -> bool {
        true
    }

    fn messages_cover_full_request(&self) -> bool {
        false
    }
}

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

// GuardrailContract returns impl Future, so the unit adapters need one concrete guardrail type; this is its object-safe form
pub trait DynGuardrail: Sync {
    fn apply_guardrail<'call>(
        &'call self,
        inputs: GuardrailApiInputs,
        request_data: &'call mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&'call mut GuardrailLogTarget>,
    ) -> Pin<Box<dyn Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send + 'call>>;

    fn get_config_model(&self) -> Option<GuardrailValue>;

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>>;

    fn supports_tool_result_scan(&self) -> bool;

    fn messages_cover_full_request(&self) -> bool;
}

impl<G: GuardrailContract> DynGuardrail for G {
    fn apply_guardrail<'call>(
        &'call self,
        inputs: GuardrailApiInputs,
        request_data: &'call mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&'call mut GuardrailLogTarget>,
    ) -> Pin<Box<dyn Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send + 'call>>
    {
        Box::pin(GuardrailContract::apply_guardrail(
            self,
            inputs,
            request_data,
            input_type,
            logging_obj,
        ))
    }

    fn get_config_model(&self) -> Option<GuardrailValue> {
        GuardrailContract::get_config_model(self)
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        GuardrailContract::get_supported_event_hooks(self)
    }

    fn supports_tool_result_scan(&self) -> bool {
        GuardrailContract::supports_tool_result_scan(self)
    }

    fn messages_cover_full_request(&self) -> bool {
        GuardrailContract::messages_cover_full_request(self)
    }
}

// The guardrail handle the six unit adapters carry; callers erase any Send + Sync GuardrailContract into it
#[derive(Clone)]
pub struct AnyGuardrail(pub Arc<dyn DynGuardrail + Send + Sync>);

impl GuardrailContract for AnyGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        let guardrail = Arc::clone(&self.0);
        async move {
            guardrail
                .apply_guardrail(inputs, request_data, input_type, logging_obj)
                .await
        }
    }

    fn get_config_model(&self) -> Option<GuardrailValue> {
        self.0.get_config_model()
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        self.0.get_supported_event_hooks()
    }

    fn supports_tool_result_scan(&self) -> bool {
        self.0.supports_tool_result_scan()
    }

    fn messages_cover_full_request(&self) -> bool {
        self.0.messages_cover_full_request()
    }
}

mod tests;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
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
}

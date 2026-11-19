pub mod call;
pub mod context;
pub mod failure;
pub mod headers;
pub mod helpers;
pub mod payload;

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::inputs::GuardrailLogTarget;

pub type Object = BTreeMap<String, GuardrailValue>;

// The request_data and logging object of LiteLLM's apply_guardrail call, for the monitoring helpers
pub struct RequestContext<'call> {
    pub request_data: &'call mut Object,
    pub log_target: Option<&'call mut GuardrailLogTarget>,
}

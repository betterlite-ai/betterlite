use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::presidio_actions::PresidioAnonymizer;
use crate::presidio_actions::check::CheckPiiCall;
use crate::presidio_actions::request_data::{PresidioPerRequestConfig, RequestData};
use crate::presidio_analysis::PresidioAnalyzer;

pub mod anthropic;
pub mod error;
pub mod logging;
pub mod model;
pub mod post_call;
pub mod wire;

mod log_merge;
mod messages;
mod runtime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Mask,
    Unmask,
}

#[derive(Debug, Clone, Copy)]
pub struct HookContext<'context> {
    pub analyzer: &'context PresidioAnalyzer,
    pub anonymizer: &'context PresidioAnonymizer,
    pub pii_entities_config: &'context BTreeMap<String, PiiAction>,
    pub dynamic_params: &'context BTreeMap<String, GuardrailValue>,
    // litellm.output_parse_pii, ORed with the instance setting in async_post_call_success_hook
    pub global_output_parse_pii: bool,
}

impl HookContext<'_> {
    #[must_use]
    pub const fn check_call<'call>(
        &'call self,
        text: &'call str,
        presidio_config: Option<&'call PresidioPerRequestConfig>,
        request_data: &'call mut RequestData,
    ) -> CheckPiiCall<'call> {
        CheckPiiCall {
            text,
            output_parse_pii: false,
            presidio_config,
            dynamic_params: self.dynamic_params,
            pii_entities_config: self.pii_entities_config,
            request_data,
        }
    }
}

mod tests;

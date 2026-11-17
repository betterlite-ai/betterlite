use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use self::error::PresidioResponseError;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::presidio_actions::PresidioAnonymizer;
use crate::presidio_analysis::PresidioAnalyzer;

pub mod anthropic;
pub mod error;
pub mod logging;
pub mod model;
pub mod post_call;
pub mod wire;

mod log_merge;
pub(crate) mod messages;
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

impl From<PresidioResponseError> for GuardrailFailure {
    fn from(error: PresidioResponseError) -> Self {
        match error {
            PresidioResponseError::Actions(actions) => Self::from(actions),
            other @ (PresidioResponseError::MessagesNotList
            | PresidioResponseError::MessagesJson { .. }
            | PresidioResponseError::BodyJson { .. }
            | PresidioResponseError::Runtime { .. }
            | PresidioResponseError::Client { .. }
            | PresidioResponseError::HookThreadPanicked) => Self::Http {
                status: other.status(),
                detail: other.to_string(),
            },
        }
    }
}

mod tests;

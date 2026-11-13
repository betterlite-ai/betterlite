use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;
use secrecy::SecretString;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::advisory::DEFAULT_ADVISORY_MESSAGE;
use crate::guardrail_message_scope::scoped_indices::GuardrailSkipFlags;
use crate::lakera_pii::wire::LakeraResponse;

use super::advisory;
use super::error::LakeraV2Error;

mod call;
mod construction;
mod update;

pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 3] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
];

pub const DEFAULT_API_BASE: &str = "https://api.lakera.ai";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LakeraV2Flags {
    pub payload: GuardrailFlag,
    pub breakdown: GuardrailFlag,
    pub dev_info: GuardrailFlag,
}

pub struct LakeraV2Guardrail {
    pub base: GuardrailBase,
    pub client: reqwest::Client,
    pub api_key: SecretString,
    pub api_base: String,
    pub project_id: Option<String>,
    pub metadata: Option<BTreeMap<String, GuardrailValue>>,
    pub flags: LakeraV2Flags,
    pub on_flagged: LakeraOnFlagged,
    pub advisory_system_message: Option<String>,
    pub skip_flags: GuardrailSkipFlags,
    pub timeout: Option<Duration>,
}

impl LakeraV2Guardrail {
    pub fn build_advisory_message(
        &self,
        response: Option<&LakeraResponse>,
    ) -> Result<String, LakeraV2Error> {
        let breakdown = response.and_then(|value| value.breakdown.as_deref());
        let reason = advisory::humanize_lakera_block_reasons(breakdown);
        let template = self
            .advisory_system_message
            .as_deref()
            .unwrap_or(DEFAULT_ADVISORY_MESSAGE);
        advisory::format::format_reason(template, &reason)
            .map_err(|source| LakeraV2Error::AdvisoryFormat { source })
    }
}

pub mod error;

mod construction;
mod contract;
mod data;
mod hooks;
mod tests;

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::presidio_runtime::Settings as RuntimeSettings;
use reqwest::Client;

use self::error::PresidioRuntimeError;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_initializers::presidio::{PresidioArgs, PresidioRole};
use crate::presidio_actions::PresidioAnonymizer;
use crate::presidio_analysis::PresidioAnalyzer;

// The two bases LiteLLM reads through get_secret; B8666 resolves them with B8500's lookup and hands them in
#[derive(Debug, Clone, Default)]
pub struct PresidioDefaults {
    pub analyzer_api_base: Option<String>,
    pub anonymizer_api_base: Option<String>,
}

pub struct PresidioGuardParams<'settings> {
    pub defaults: PresidioDefaults,
    pub settings: &'settings RuntimeSettings,
    pub logging_only: bool,
    // every callback B3681 builds has a role; only the roleless legacy callback leaves this None
    pub callback_role: Option<PresidioRole>,
}

pub struct PresidioGuard {
    base: GuardrailBase,
    analyzer: PresidioAnalyzer,
    anonymizer: PresidioAnonymizer,
    pii_entities_config: BTreeMap<String, PiiAction>,
    // litellm_settings.output_parse_pii, which B3806's post-call hook ORs with the instance setting
    global_output_parse_pii: bool,
    callback_role: Option<PresidioRole>,
}

impl PresidioGuard {
    pub fn new(
        args: &PresidioArgs,
        params: PresidioGuardParams<'_>,
        mut base: GuardrailBase,
        client: Client,
    ) -> Result<Self, PresidioRuntimeError> {
        let built = construction::build(args, &params, &mut base, client)?;
        Ok(Self {
            base,
            analyzer: built.analyzer,
            anonymizer: built.anonymizer,
            pii_entities_config: built.pii_entities_config,
            global_output_parse_pii: params.settings.output_parse_pii,
            callback_role: params.callback_role,
        })
    }
}

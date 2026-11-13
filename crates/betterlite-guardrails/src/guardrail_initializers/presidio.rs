use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use serde::Deserialize;

use super::error::InitializerError;
use super::extras;
use super::presidio_mode::{is_mcp_only_mode, presidio_output_mode};

mod tests;

const FILTER_SCOPE: &str = "presidio_filter_scope";
const CHUNK_SIZE: &str = "presidio_analyze_chunk_size_bytes";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresidioRole {
    Scan,
    Restore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PresidioScope {
    Input,
    Output,
    Both,
}

#[derive(Debug, Clone)]
pub struct PresidioArgs {
    pub role: PresidioRole,
    pub name: String,
    pub event_hook: LitellmParamsMode,
    pub output_parse_pii: Option<bool>,
    pub presidio_ad_hoc_recognizers: Option<String>,
    pub mock_redacted_text: Option<BTreeMap<String, GuardrailValue>>,
    pub default_on: GuardrailFlag,
    pub pii_entities_config: Option<BTreeMap<String, PiiAction>>,
    pub presidio_score_thresholds: Option<BTreeMap<String, f64>>,
    pub presidio_analyzer_api_base: Option<String>,
    pub presidio_anonymizer_api_base: Option<String>,
    pub presidio_language: Option<String>,
    pub presidio_entities_deny_list: Option<Vec<String>>,
    pub apply_to_output: GuardrailFlag,
    pub mask_response_content: GuardrailFlag,
    pub timeout: Option<f64>,
    pub analyze_chunk_size_bytes: Option<u64>,
}

pub fn initialize_presidio(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<Vec<PresidioArgs>, InitializerError> {
    let explicit_scope = extras::read::<PresidioScope>(&params.base.extra, FILTER_SCOPE, None)?;
    let scope = match explicit_scope {
        Some(scope) => scope,
        None if is_mcp_only_mode(&params.mode) => PresidioScope::Input,
        None => PresidioScope::Both,
    };
    let scan = scan_args(params, guardrail_name)?;
    let runs_input = matches!(scope, PresidioScope::Input | PresidioScope::Both);
    let runs_output = matches!(scope, PresidioScope::Output | PresidioScope::Both);
    let mut callbacks = Vec::new();
    if runs_input {
        callbacks.push(scan.clone());
    }
    if runs_input && scan.output_parse_pii == Some(true) {
        let mut restore = scan.clone();
        restore.role = PresidioRole::Restore;
        restore.output_parse_pii = Some(true);
        restore.event_hook = presidio_output_mode(&params.mode, true);
        callbacks.push(restore);
    }
    if runs_output {
        let mut mask = scan;
        mask.apply_to_output = true;
        mask.output_parse_pii = Some(false);
        mask.mask_response_content = true;
        mask.event_hook = presidio_output_mode(&params.mode, explicit_scope.is_some());
        callbacks.push(mask);
    }
    Ok(callbacks)
}

fn scan_args(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<PresidioArgs, InitializerError> {
    let extra = &params.base.extra;
    Ok(PresidioArgs {
        role: PresidioRole::Scan,
        name: guardrail_name.unwrap_or_default().to_owned(),
        event_hook: params.mode.clone(),
        output_parse_pii: extras::read(extra, "output_parse_pii", None)?,
        presidio_ad_hoc_recognizers: extras::read(extra, "presidio_ad_hoc_recognizers", None)?,
        mock_redacted_text: extras::read(extra, "mock_redacted_text", None)?,
        default_on: params.base.fields.default_on.unwrap_or_default(),
        pii_entities_config: extras::read(extra, "pii_entities_config", None)?,
        presidio_score_thresholds: extras::read(extra, "presidio_score_thresholds", None)?,
        presidio_analyzer_api_base: extras::read(extra, "presidio_analyzer_api_base", None)?,
        presidio_anonymizer_api_base: extras::read(extra, "presidio_anonymizer_api_base", None)?,
        presidio_language: extras::read(extra, "presidio_language", Some("en".to_owned()))?,
        presidio_entities_deny_list: extras::read(extra, "presidio_entities_deny_list", None)?,
        apply_to_output: false,
        mask_response_content: false,
        timeout: params.base.fields.timeout,
        // LiteLLM's handler swaps a non-positive size for its default, so a negative one reads as absent
        analyze_chunk_size_bytes: extras::read::<i64>(extra, CHUNK_SIZE, None)?
            .and_then(|size| u64::try_from(size).ok()),
    })
}

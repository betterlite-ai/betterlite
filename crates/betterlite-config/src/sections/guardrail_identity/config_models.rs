use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::enums::PiiAction;
use super::item::GuardrailValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardrailParamUITypes {
    #[serde(rename = "bool")]
    Bool,
    #[serde(rename = "str")]
    Str,
    #[serde(rename = "multiselect")]
    Multiselect,
    #[serde(rename = "percentage")]
    Percentage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PresidioConfigUi {
    #[serde(default)]
    pub presidio_analyzer_api_base: Option<String>,
    #[serde(default)]
    pub presidio_anonymizer_api_base: Option<String>,
    #[serde(default)]
    pub presidio_filter_scope: Option<String>,
    #[serde(default)]
    pub output_parse_pii: Option<bool>,
    pub presidio_language: Option<String>,
    #[serde(default)]
    pub presidio_run_on: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default = "default_presidio_model", deny_unknown_fields)]
pub struct PresidioConfigModel {
    #[serde(default)]
    pub presidio_analyzer_api_base: Option<String>,
    #[serde(default)]
    pub presidio_anonymizer_api_base: Option<String>,
    #[serde(default)]
    pub presidio_filter_scope: Option<String>,
    #[serde(default)]
    pub output_parse_pii: Option<bool>,
    pub presidio_language: Option<String>,
    #[serde(default)]
    pub presidio_run_on: Option<String>,
    #[serde(default)]
    pub pii_entities_config: Option<BTreeMap<String, PiiAction>>,
    #[serde(default)]
    pub presidio_score_thresholds: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    pub presidio_entities_deny_list: Option<Vec<String>>,
    #[serde(default)]
    pub presidio_ad_hoc_recognizers: Option<String>,
    #[serde(default)]
    #[serde(rename = "presidio_analyze_chunk_size_bytes")]
    pub analyze_chunk_size_bytes: Option<i64>,
    #[serde(default)]
    pub mock_redacted_text: Option<BTreeMap<String, GuardrailValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LassoGuardrailConfigModel {
    #[serde(default)]
    pub lasso_user_id: Option<String>,
    #[serde(default)]
    pub lasso_conversation_id: Option<String>,
    pub mask: Option<bool>,
}

impl Default for PresidioConfigUi {
    fn default() -> Self {
        let defaults = default_presidio_fields();
        let (analyzer, anonymizer, scope, parse, language, run_on) = defaults;
        Self {
            presidio_analyzer_api_base: analyzer,
            presidio_anonymizer_api_base: anonymizer,
            presidio_filter_scope: scope,
            output_parse_pii: parse,
            presidio_language: language,
            presidio_run_on: run_on,
        }
    }
}

fn default_presidio_model() -> PresidioConfigModel {
    let defaults = default_presidio_fields();
    let (analyzer, anonymizer, scope, parse, language, run_on) = defaults;
    PresidioConfigModel {
        presidio_analyzer_api_base: analyzer,
        presidio_anonymizer_api_base: anonymizer,
        presidio_filter_scope: scope,
        output_parse_pii: parse,
        presidio_language: language,
        presidio_run_on: run_on,
        pii_entities_config: None,
        presidio_score_thresholds: None,
        presidio_entities_deny_list: None,
        presidio_ad_hoc_recognizers: None,
        analyze_chunk_size_bytes: None,
        mock_redacted_text: None,
    }
}

type PresidioDefaults = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<bool>,
    Option<String>,
    Option<String>,
);

fn default_presidio_fields() -> PresidioDefaults {
    let analyzer = None;
    let anonymizer = None;
    let scope = None;
    let parse = None;
    let language = Some(String::from("en"));
    let run_on = None;
    (analyzer, anonymizer, scope, parse, language, run_on)
}

impl Default for LassoGuardrailConfigModel {
    fn default() -> Self {
        Self {
            lasso_user_id: None,
            lasso_conversation_id: None,
            mask: Some(false),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeepKeepGuardrailConfigModel {
    #[serde(default)]
    pub deepkeep_firewall_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardrailConfigModel<T> {
    #[serde(default)]
    pub optional_params: Option<T>,
}

pub trait GuardrailConfigModelIdentity {
    fn ui_friendly_name() -> &'static str;
}

impl GuardrailConfigModelIdentity for PresidioConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Presidio Guardrail"
    }
}

impl GuardrailConfigModelIdentity for LassoGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Lasso Guardrail"
    }
}

impl GuardrailConfigModelIdentity for DeepKeepGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "DeepKeep AI Firewall"
    }
}

use betterlite_config::sections::content_filter_records::{
    BlockedWord, ContentFilterCategoryConfig, ContentFilterPattern,
};
use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::base::OnViolation;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;

#[derive(Debug, Clone)]
pub struct ContentFilterArgs {
    pub name: String,
    pub guardrail_id: Option<String>,
    pub policy_template: Option<String>,
    pub patterns: Option<Vec<ContentFilterPattern>>,
    pub blocked_words: Option<Vec<BlockedWord>>,
    pub blocked_words_file: Option<String>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub categories: Option<Vec<ContentFilterCategoryConfig>>,
    pub severity_threshold: String,
    pub router_present: bool,
    pub image_model: Option<String>,
    pub competitor_intent_config: Option<OpaqueParams>,
    pub end_session_after_n_fails: Option<i64>,
    pub on_violation: Option<OnViolation>,
    pub realtime_violation_message: Option<String>,
}

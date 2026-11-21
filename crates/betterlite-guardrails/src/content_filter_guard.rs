use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_filter_records::ContentFilterAction;
use betterlite_config::sections::content_filter_records::overrides::CompetitorIntentResult;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use indexmap::IndexMap;

use crate::content_filter::competitor_intent_airline::AirlineCompetitorIntentChecker;
use crate::content_filter::competitor_intent_base::BaseCompetitorIntentChecker;
use crate::guardrail_base::GuardrailBase;
use crate::pattern_matching::CompiledPattern;

mod apply;
pub mod build;
pub mod error;
mod record;
mod tests;

pub const GUARDRAIL_PROVIDER: &str = "litellm_content_filter";
pub const PATTERN_REDACTION_FORMAT: &str = "[{pattern_name}_REDACTED]";
pub const KEYWORD_REDACTION_TAG: &str = "[KEYWORD_REDACTED]";
pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 6] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::RealtimeInputTranscription,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::PostMcpCall,
];

pub enum CompetitorIntentChecker {
    Generic(BaseCompetitorIntentChecker),
    Airline(AirlineCompetitorIntentChecker),
}

impl CompetitorIntentChecker {
    #[must_use]
    pub fn run(&self, text: &str) -> CompetitorIntentResult {
        match self {
            Self::Generic(checker) => checker.run(text),
            Self::Airline(checker) => checker.run(text),
        }
    }
}

pub struct ContentFilterGuardrail<R> {
    pub base: GuardrailBase,
    pub guardrail_provider: String,
    pub config_guardrail_id: Option<String>,
    pub config_policy_template: Option<String>,
    pub pattern_redaction_format: String,
    pub keyword_redaction_tag: String,
    pub severity_threshold: String,
    pub router: Option<R>,
    pub image_model: Option<String>,
    pub loaded_categories: CategoryLoadResult,
    pub competitor_intent_checker: Option<CompetitorIntentChecker>,
    pub refuse_message_template: Option<String>,
    pub reframe_message_template: Option<String>,
    pub compiled_patterns: Vec<CompiledPattern>,
    pub blocked_words: IndexMap<String, (ContentFilterAction, Option<String>)>,
}

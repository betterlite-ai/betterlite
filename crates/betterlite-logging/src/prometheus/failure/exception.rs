use betterlite_error::normalize::class::Class;
use betterlite_error::rate_limit::ProxyRateLimitError;

use crate::logging_metadata::exception::{ExceptionFacts, semantic_class};

// The exception facts the failure labels read, shaped the way LiteLLM's getattr calls read them.
#[derive(Clone, Default)]
pub struct LabelException {
    pub class_name: String,
    pub class: Class,
    pub message: Option<String>,
    pub status_code: Option<String>,
    pub code: Option<String>,
    pub llm_provider: Option<String>,
    pub category: Option<String>,
    pub rate_limit_type: Option<String>,
    pub legacy_class_name: Option<String>,
}

impl LabelException {
    #[must_use]
    pub fn from_facts(facts: &ExceptionFacts) -> Self {
        Self {
            class_name: facts.class_name.clone(),
            class: semantic_class(facts),
            message: facts.message.clone(),
            status_code: facts.status_code.map(|code| code.to_string()),
            code: facts.code.clone(),
            llm_provider: facts.llm_provider.clone(),
            category: facts.category.clone(),
            rate_limit_type: facts.rate_limit_type.clone(),
            legacy_class_name: None,
        }
    }

    // A proxy-side 429 pins the exception_class label to HTTPException, which older dashboards key on.
    #[must_use]
    pub fn from_proxy_rate_limit(facts: &ExceptionFacts) -> Self {
        Self {
            legacy_class_name: Some(ProxyRateLimitError::PROMETHEUS_EXCEPTION_CLASS.to_owned()),
            ..Self::from_facts(facts)
        }
    }
}

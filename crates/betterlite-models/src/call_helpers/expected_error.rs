use serde::Deserialize;

use crate::decisions::Json;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ClientErrorFacts {
    pub code: Option<Json>,
    pub status_code: Option<Json>,
    pub llm_provider: Option<String>,
    pub category: Option<String>,
    pub http_exception: bool,
    pub base_llm_exception: bool,
}

#[must_use]
pub fn is_expected_client_error(facts: Option<&ClientErrorFacts>) -> bool {
    let Some(given) = facts else {
        return false;
    };
    if is_provider_originated(given) {
        return false;
    }
    let Some(status_source) = given
        .code
        .as_ref()
        .filter(|value| !matches!(value, Json::Null))
        .or_else(|| {
            given
                .status_code
                .as_ref()
                .filter(|value| !matches!(value, Json::Null))
        })
    else {
        return false;
    };
    let Some(status) = status_number(status_source) else {
        return false;
    };
    (400_i64..500_i64).contains(&status)
}

fn is_provider_originated(facts: &ClientErrorFacts) -> bool {
    let limit_rejection = matches!(
        facts.category.as_deref(),
        Some("litellm_rate_limit" | "litellm_batch_rate_limit")
    );
    if limit_rejection || facts.http_exception {
        return false;
    }
    facts
        .llm_provider
        .as_deref()
        .is_some_and(|provider| !provider.is_empty())
        || facts.base_llm_exception
}

fn status_number(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number.as_i64(),
        Json::Text(text) => text.trim().parse::<i64>().ok(),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

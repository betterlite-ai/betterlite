use betterlite_error::normalize::class::Class;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::prometheus::failure::exception::LabelException;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ExceptionInput {
    Scripted(ScriptedInput),
    RateLimit(RateLimitInput),
    BudgetExceeded(BudgetInput),
    ProxyRateLimit(ProxyRateLimitInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScriptedInput {
    pub(crate) message: Option<String>,
    pub(crate) status_code: Option<String>,
    pub(crate) code: Option<String>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) rate_limit_type: Option<String>,
    pub(crate) legacy_class_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RateLimitInput {
    pub(crate) message: Option<String>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) rate_limit_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BudgetInput {
    pub(crate) message: Option<String>,
    pub(crate) llm_provider: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProxyRateLimitInput {
    pub(crate) message: Option<String>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) rate_limit_type: Option<String>,
}

const RATE_LIMIT_MESSAGE_PREFIX: &str = "litellm.RateLimitError: ";
const VENDOR_RATE_LIMIT: &str = "vendor_rate_limit";
const LITELLM_RATE_LIMIT: &str = "litellm_rate_limit";
const PROXY_LLM_PROVIDER: &str = "litellm_proxy";
const BUDGET_DEFAULT_MESSAGE: &str = "Budget has been exceeded! Current cost: 0.0, Max budget: 0.0";

pub(crate) fn label_exception(input: &ExceptionInput) -> LabelException {
    match input {
        ExceptionInput::Scripted(scripted) => LabelException {
            class_name: String::from("ScriptedError"),
            class: Class::Other,
            message: scripted.message.clone(),
            status_code: scripted.status_code.clone(),
            code: scripted.code.clone(),
            llm_provider: scripted.llm_provider.clone(),
            category: scripted.category.clone(),
            rate_limit_type: scripted.rate_limit_type.clone(),
            legacy_class_name: scripted.legacy_class_name.clone(),
        },
        ExceptionInput::RateLimit(rate_limit) => rate_limit_exception(rate_limit),
        ExceptionInput::BudgetExceeded(budget) => LabelException {
            class_name: String::from("BudgetExceededError"),
            class: Class::BudgetExceededError,
            message: Some(budget_message(budget.message.as_deref())),
            status_code: Some(String::from("422")),
            code: None,
            llm_provider: Some(budget.llm_provider.clone().unwrap_or_default()),
            category: Some(String::from(LITELLM_RATE_LIMIT)),
            rate_limit_type: Some(String::from("budget")),
            legacy_class_name: None,
        },
        ExceptionInput::ProxyRateLimit(proxy) => proxy_rate_limit_exception(proxy),
    }
}

pub(crate) fn label_pair(
    exception: Option<&ExceptionInput>,
    kwargs_exception: Option<&ExceptionInput>,
) -> (Option<LabelException>, Option<LabelException>) {
    let direct = exception.map(label_exception);
    let from_kwargs = kwargs_exception.map(label_exception);
    (direct, from_kwargs)
}

fn rate_limit_exception(rate_limit: &RateLimitInput) -> LabelException {
    LabelException {
        class_name: String::from("RateLimitError"),
        class: Class::RateLimitError,
        message: Some(prefixed_message(rate_limit.message.as_deref())),
        status_code: Some(String::from("429")),
        code: Some(String::from("429")),
        llm_provider: rate_limit.llm_provider.clone(),
        category: Some(
            rate_limit
                .category
                .clone()
                .unwrap_or_else(|| String::from(VENDOR_RATE_LIMIT)),
        ),
        rate_limit_type: rate_limit.rate_limit_type.clone(),
        legacy_class_name: None,
    }
}

fn proxy_rate_limit_exception(proxy: &ProxyRateLimitInput) -> LabelException {
    let llm_provider = proxy
        .llm_provider
        .clone()
        .filter(|provider| !provider.is_empty())
        .unwrap_or_else(|| String::from(PROXY_LLM_PROVIDER));
    let category = proxy
        .category
        .clone()
        .unwrap_or_else(|| String::from(LITELLM_RATE_LIMIT));
    let facts = ExceptionFacts {
        class_name: String::from("ProxyRateLimitError"),
        class: None,
        message: Some(prefixed_message(proxy.message.as_deref())),
        code: Some(String::from("429")),
        status_code: Some(429),
        response_status_code: None,
        llm_provider: Some(llm_provider),
        category: Some(category),
        rate_limit_type: proxy.rate_limit_type.clone(),
        budget_entity_type: None,
        budget_entity_id: None,
        budget_limit: None,
        budget_spend: None,
        proxy_type: None,
        headers: None,
        response_headers: None,
        litellm_response_headers: None,
        traceback_lines: Vec::new(),
    };
    LabelException::from_proxy_rate_limit(&facts)
}

// RateLimitError.__init__ prefixes the message it was given.
fn prefixed_message(message: Option<&str>) -> String {
    let text = message.unwrap_or_default();
    format!("{RATE_LIMIT_MESSAGE_PREFIX}{text}")
}

// BudgetExceededError replaces an empty message with its own summary of the zero-cost budget.
fn budget_message(message: Option<&str>) -> String {
    message
        .filter(|text| !text.is_empty())
        .unwrap_or(BUDGET_DEFAULT_MESSAGE)
        .to_owned()
}

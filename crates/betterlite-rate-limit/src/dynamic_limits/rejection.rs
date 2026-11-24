use std::iter::once;

use betterlite_auth::provider_resolution::resolve_rate_limit_provider;
use betterlite_error::rate_limit::{
    InvalidRateLimitHeader, ProxyRateLimitError, ProxyRateLimitErrorInit, RateLimitDimension,
};
use betterlite_redact::structured::Payload;

use crate::provider_attribution::RateLimitErrorAttribution;

// an f-string renders a None as this word, which the detail keeps for a value LiteLLM could not compute
const NONE_TEXT: &str = "None";

// which check refused the request, named as the detail text names the limit it reports
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExceededLimit {
    Tokens,
    Requests,
}

impl ExceededLimit {
    // RateLimitType.TOKENS and RateLimitType.REQUESTS as betterlite's dimension carries them
    const fn dimension(self) -> RateLimitDimension {
        match self {
            Self::Tokens => RateLimitDimension::Tokens,
            Self::Requests => RateLimitDimension::Requests,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Tokens => "TPM",
            Self::Requests => "RPM",
        }
    }
}

// the facts a refusal's detail names: which check ran out, its share, the model's remaining limit, the active projects and the key
pub struct LimitCheck<'check> {
    pub exceeded: ExceededLimit,
    pub quota: Option<i64>,
    pub model_limit: Option<i64>,
    pub active_projects: Option<i64>,
    pub api_key: Option<&'check str>,
}

// what async_pre_call_hook refuses with: the 429 to forward, or the header value HTTP cannot carry
#[derive(Debug, thiserror::Error)]
pub enum DynamicLimitError {
    #[error("{0}")]
    Rejected(Box<ProxyRateLimitError>),
    #[error(transparent)]
    Header(#[from] InvalidRateLimitHeader),
}

// async_pre_call_hook's ProxyRateLimitError: the resolved model pair, the dimension and the detail message
pub fn dynamic_rate_limit_error(
    check: &LimitCheck<'_>,
    attribution: &RateLimitErrorAttribution<'_>,
) -> Result<ProxyRateLimitError, InvalidRateLimitHeader> {
    let (model, llm_provider) = resolve_rate_limit_provider(
        attribution.requested_model,
        attribution.native,
        attribution.router,
    );
    let init = ProxyRateLimitErrorInit {
        detail: Payload::Object(
            once((
                String::from("error"),
                Payload::String(detail(check, check.exceeded)),
            ))
            .collect(),
        ),
        headers: None,
        category: None,
        rate_limit_type: Some(check.exceeded.dimension()),
        model: Some(model),
        llm_provider: Some(llm_provider),
    };
    ProxyRateLimitError::new(init)
}

// LiteLLM's f-string: the key, the quota that ran out, the model's remaining limit and the active key count
fn detail(check: &LimitCheck<'_>, exceeded: ExceededLimit) -> String {
    let api_key = check.api_key.unwrap_or(NONE_TEXT);
    let label = exceeded.label();
    // a Python int writes its digits into the text and a None writes None
    let quota = check
        .quota
        .map_or_else(|| String::from(NONE_TEXT), |limit| limit.to_string());
    let model_limit = check
        .model_limit
        .map_or_else(|| String::from(NONE_TEXT), |limit| limit.to_string());
    let active_projects = check
        .active_projects
        .map_or_else(|| String::from(NONE_TEXT), |limit| limit.to_string());
    format!(
        "Key={api_key} over available {label}={quota}. Model {label}={model_limit}, Active keys={active_projects}"
    )
}

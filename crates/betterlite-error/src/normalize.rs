use regex::{RegexSet, RegexSetBuilder};

pub mod class;
pub mod provider;
use crate::rate_limit::RateLimitDimension;
use class::Class;

pub struct ErrorFacts {
    pub proxy_type: Option<String>,
    pub message: String,
    pub class: Class,
    pub status_code: Option<u16>,
    pub rate_limit_type: Option<RateLimitDimension>,
}

#[derive(Debug, thiserror::Error)]
pub enum NormalizeError {
    #[error("error normalization patterns could not be compiled")]
    Patterns {
        #[source]
        source: regex::Error,
    },
}

pub struct Normalizer {
    messages: RegexSet,
}

const MESSAGE_PATTERNS: &[(&str, &str)] = &[
    (r"exceeded[^\n]*budget", "429_BUDGET_EXCEEDED"),
    (
        "upstream passthrough request failed",
        "500_UPSTREAM_PASSTHROUGH",
    ),
    (
        "budget has been exceeded|max budget|crossed budget",
        "429_BUDGET_EXCEEDED",
    ),
    (
        "no healthy deployments?|no deployments available",
        "429_NO_HEALTHY_DEPLOYMENTS",
    ),
    (
        "not allowed to access model due to tags configuration",
        "403_MODEL_ACCESS_DENIED",
    ),
    (
        "is not supported for provider|not implemented",
        "500_UNSUPPORTED_OPERATION",
    ),
    (
        r"context window|context length|(prompt|input) is too long|tokens? ?> ?\d+ ?maximum",
        "400_CONTEXT_WINDOW_EXCEEDED",
    ),
    (
        "missing required parameter|field required",
        "400_MISSING_REQUIRED_PARAMETER",
    ),
    (
        "overloaded|unable to process your request",
        "503_PROVIDER_OVERLOADED",
    ),
    (
        "connection error|APIConnectionError|TransferEncodingError|payload is not completed|connection reset|peer closed connection|incomplete chunked read",
        "500_PROVIDER_CONNECTION_ERROR",
    ),
    ("timed? ?out", "408_UPSTREAM_TIMEOUT"),
];

impl Normalizer {
    pub fn new() -> Result<Self, NormalizeError> {
        let messages = RegexSetBuilder::new(MESSAGE_PATTERNS.iter().map(|(pattern, _)| pattern))
            .case_insensitive(true)
            .build()
            .map_err(|source| NormalizeError::Patterns { source })?;
        Ok(Self { messages })
    }

    #[must_use]
    pub fn normalize(&self, facts: Option<&ErrorFacts>) -> Option<&'static str> {
        let error = facts?;
        let by_message = self
            .messages
            .matches(&error.message)
            .iter()
            .next()
            .and_then(|index| MESSAGE_PATTERNS.get(index).map(|(_, code)| *code));
        let by_proxy_type = match error.proxy_type.as_deref() {
            Some("budget_exceeded") => Some("429_BUDGET_EXCEEDED"),
            Some(
                "auth_error"
                | "expired_key"
                | "token_not_found_in_db"
                | "auth_provider_unavailable",
            ) => Some("401_AUTHENTICATION_FAILED"),
            Some(
                "key_model_access_denied"
                | "team_model_access_denied"
                | "user_model_access_denied"
                | "org_model_access_denied"
                | "project_model_access_denied"
                | "agent_model_access_denied",
            ) => Some("403_MODEL_ACCESS_DENIED"),
            Some(
                "key_vector_store_access_denied"
                | "team_vector_store_access_denied"
                | "org_vector_store_access_denied"
                | "tool_access_denied"
                | "team_member_permission_error",
            ) => Some("403_PERMISSION_DENIED"),
            Some("not_found_error") => Some("404_RESOURCE_NOT_FOUND"),
            _ => None,
        };
        by_proxy_type
            .or(by_message)
            .or_else(|| error.class.code(error.rate_limit_type))
            .or_else(|| router_code(&error.message))
            .or_else(|| Some(status_code(error.status_code)))
    }
}

fn router_code(message: &str) -> Option<&'static str> {
    let lower = message.to_lowercase();
    if lower.contains("no fallback model group found") {
        Some("500_ROUTER_NO_FALLBACK")
    } else if lower.contains("error doing the fallback") || lower.contains("midstreamfallbackerror")
    {
        Some("500_ROUTER_FALLBACK_FAILURE")
    } else {
        None
    }
}

const fn status_code(status: Option<u16>) -> &'static str {
    match status {
        Some(400) => "400_INVALID_REQUEST",
        Some(401) => "401_AUTHENTICATION_FAILED",
        Some(403) => "403_PERMISSION_DENIED",
        Some(404) => "404_RESOURCE_NOT_FOUND",
        Some(408 | 504) => "408_UPSTREAM_TIMEOUT",
        Some(422) => "400_INVALID_PARAMETER_VALUE",
        Some(429) => "429_RATE_LIMIT_EXCEEDED",
        Some(500 | 502) => "500_PROVIDER_INTERNAL_ERROR",
        Some(503) => "503_PROVIDER_OVERLOADED",
        _ => "UNCLASSIFIED",
    }
}

mod tests;

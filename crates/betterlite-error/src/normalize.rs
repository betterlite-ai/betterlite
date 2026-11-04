#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorFacts {
    pub proxy_type: Option<String>,
    pub message: String,
    pub class: Option<String>,
    pub status_code: Option<u16>,
    pub rate_limit_type: Option<String>,
}

enum StringMatch {
    Equal(&'static str),
    Contains(&'static [&'static str]),
}

struct ErrorCodePattern {
    matcher: StringMatch,
    code: &'static str,
}

const PROXY_PATTERNS: &[ErrorCodePattern] = &[
    ErrorCodePattern {
        matcher: StringMatch::Equal("budget_exceeded"),
        code: "429_BUDGET_EXCEEDED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("auth_error"),
        code: "401_AUTHENTICATION_FAILED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("expired_key"),
        code: "401_AUTHENTICATION_FAILED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("token_not_found_in_db"),
        code: "401_AUTHENTICATION_FAILED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("auth_provider_unavailable"),
        code: "401_AUTHENTICATION_FAILED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("key_model_access_denied"),
        code: "403_MODEL_ACCESS_DENIED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("team_model_access_denied"),
        code: "403_MODEL_ACCESS_DENIED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("user_model_access_denied"),
        code: "403_MODEL_ACCESS_DENIED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("org_model_access_denied"),
        code: "403_MODEL_ACCESS_DENIED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("project_model_access_denied"),
        code: "403_MODEL_ACCESS_DENIED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("agent_model_access_denied"),
        code: "403_MODEL_ACCESS_DENIED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("key_vector_store_access_denied"),
        code: "403_PERMISSION_DENIED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("team_vector_store_access_denied"),
        code: "403_PERMISSION_DENIED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("org_vector_store_access_denied"),
        code: "403_PERMISSION_DENIED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("tool_access_denied"),
        code: "403_PERMISSION_DENIED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("team_member_permission_error"),
        code: "403_PERMISSION_DENIED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Equal("not_found_error"),
        code: "404_RESOURCE_NOT_FOUND",
    },
];

const MESSAGE_PATTERNS: &[ErrorCodePattern] = &[
    ErrorCodePattern {
        matcher: StringMatch::Contains(&["upstream passthrough request failed"]),
        code: "500_UPSTREAM_PASSTHROUGH",
    },
    ErrorCodePattern {
        matcher: StringMatch::Contains(&["no healthy deployment", "no deployments available"]),
        code: "429_NO_HEALTHY_DEPLOYMENTS",
    },
    ErrorCodePattern {
        matcher: StringMatch::Contains(&["not allowed to access model due to tags configuration"]),
        code: "403_MODEL_ACCESS_DENIED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Contains(&["not implemented", "not supported for provider"]),
        code: "500_UNSUPPORTED_OPERATION",
    },
    ErrorCodePattern {
        matcher: StringMatch::Contains(&[
            "context window",
            "context length",
            "input is too long",
            "prompt is too long",
        ]),
        code: "400_CONTEXT_WINDOW_EXCEEDED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Contains(&["missing required parameter", "field required"]),
        code: "400_MISSING_REQUIRED_PARAMETER",
    },
    ErrorCodePattern {
        matcher: StringMatch::Contains(&["overloaded", "unable to process your request"]),
        code: "503_PROVIDER_OVERLOADED",
    },
    ErrorCodePattern {
        matcher: StringMatch::Contains(&[
            "connection error",
            "connection reset",
            "apiconnectionerror",
        ]),
        code: "500_PROVIDER_CONNECTION_ERROR",
    },
    ErrorCodePattern {
        matcher: StringMatch::Contains(&["timed out", "timeout"]),
        code: "408_UPSTREAM_TIMEOUT",
    },
];

const ROUTER_PATTERNS: &[ErrorCodePattern] = &[
    ErrorCodePattern {
        matcher: StringMatch::Contains(&["no fallback model group found"]),
        code: "500_ROUTER_NO_FALLBACK",
    },
    ErrorCodePattern {
        matcher: StringMatch::Contains(&["error doing the fallback", "midstreamfallbackerror"]),
        code: "500_ROUTER_FALLBACK_FAILURE",
    },
];

fn code_for(value: &str, patterns: &[ErrorCodePattern]) -> Option<&'static str> {
    for pattern in patterns {
        let matches = match pattern.matcher {
            StringMatch::Equal(expected) => value == expected,
            StringMatch::Contains(needles) => needles.iter().any(|needle| value.contains(needle)),
        };
        if matches {
            return Some(pattern.code);
        }
    }
    None
}

fn is_exceeded_budget(line: &str) -> bool {
    line.find("exceeded").is_some_and(|position| {
        line.get(position..)
            .is_some_and(|suffix| suffix.contains("budget"))
    })
}

fn budget_message_code(message: &str) -> Option<&'static str> {
    if message.contains("max budget") || message.contains("crossed budget") {
        return Some("429_BUDGET_EXCEEDED");
    }
    message
        .split('\n')
        .any(is_exceeded_budget)
        .then_some("429_BUDGET_EXCEEDED")
}

fn class_code(facts: &ErrorFacts) -> Option<&'static str> {
    match facts.class.as_deref() {
        Some("RateLimitError") if facts.rate_limit_type.as_deref() == Some("budget") => {
            Some("429_BUDGET_EXCEEDED")
        }
        Some("RateLimitError") => Some("429_RATE_LIMIT_EXCEEDED"),
        Some("AuthenticationError") => Some("401_AUTHENTICATION_FAILED"),
        Some("PermissionDeniedError") => Some("403_PERMISSION_DENIED"),
        Some("ContextWindowExceededError") => Some("400_CONTEXT_WINDOW_EXCEEDED"),
        Some(
            "ContentPolicyViolationError" | "GuardrailRaisedException" | "BlockedPiiEntityError",
        ) => Some("400_CONTENT_POLICY_VIOLATION"),
        Some("UnsupportedParamsError") => Some("400_INVALID_PARAMETER_VALUE"),
        Some("NotFoundError") => Some("404_RESOURCE_NOT_FOUND"),
        Some("Timeout") => Some("408_UPSTREAM_TIMEOUT"),
        Some("MidStreamFallbackError") => Some("500_ROUTER_FALLBACK_FAILURE"),
        Some("APIConnectionError") => Some("500_PROVIDER_CONNECTION_ERROR"),
        Some("ServiceUnavailableError") => Some("503_PROVIDER_OVERLOADED"),
        Some("InternalServerError" | "BadGatewayError") => Some("500_PROVIDER_INTERNAL_ERROR"),
        Some("BadRequestError" | "UnprocessableEntityError") => Some("400_INVALID_REQUEST"),
        Some("NotImplementedError") => Some("500_UNSUPPORTED_OPERATION"),
        Some(
            "TypeError" | "KeyError" | "AttributeError" | "IndexError" | "RuntimeError"
            | "AssertionError" | "ZeroDivisionError",
        ) => Some("500_INTERNAL_STATE_ERROR"),
        _ => None,
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

#[must_use]
pub fn normalize_error(facts: &ErrorFacts) -> Option<&'static str> {
    if facts.proxy_type.is_none() && facts.class.is_none() && facts.status_code.is_none() {
        return None;
    }
    let message = facts.message.to_lowercase();
    facts
        .proxy_type
        .as_deref()
        .and_then(|proxy_type| code_for(proxy_type, PROXY_PATTERNS))
        .or_else(|| budget_message_code(&message))
        .or_else(|| code_for(&message, MESSAGE_PATTERNS))
        .or_else(|| class_code(facts))
        .or_else(|| code_for(&message, ROUTER_PATTERNS))
        .or_else(|| Some(status_code(facts.status_code)))
}

mod tests;

pub const GUARDRAIL_NAME: &str = "bedrock";

pub const _BEDROCK_DYNAMIC_BODY_DENYLIST: [&str; 2] = ["content", "source"];

pub const _BEDROCK_TOO_LARGE_SUBSTRINGS: [&str; 6] = [
    "text unit",
    "maximum input size",
    "content size",
    "too long",
    "too large",
    "exceeds the maximum",
];

pub const _BEDROCK_MAX_THROTTLE_RETRIES: usize = 3;

pub const _BEDROCK_BASE_BACKOFF_SECONDS: f64 = 0.5;

pub const _BEDROCK_WHITESPACE: &str = r"\s";

pub const _NO_TRACING_DETAIL: &[(&str, &str)] = &[];

// LiteLLM's _BEDROCK_INVOKE_GUARDRAIL_CHECKS_PATH, shortened to fit the 30-character name limit
pub const _BEDROCK_CHECKS_PATH: &str = "/guardrail-checks/invoke";

pub const _BEDROCK_MAX_CONTENT_BLOCKS: usize = 10;

pub const _BEDROCK_CHECKS_KNOWN_KEYS: [&str; 3] =
    ["contentFilter", "promptAttack", "sensitiveInformation"];

pub const _BEDROCK_PII_LOCATION_KEYS: [&str; 4] =
    ["beginOffset", "endOffset", "messageIndex", "contentIndex"];

pub const _CONTENT_TYPE_TO_QUALIFIER: [(&str, &str); 2] =
    [("grounding_source", "grounding_source"), ("query", "query")];

pub const _GROUNDING_TRUSTED_ROLES: [&str; 2] = ["system", "developer"];

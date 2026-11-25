pub const VERTEX_CREDENTIAL_HEADERS: [&str; 2] = ["authorization", "x-goog-api-key"];

pub const ANTHROPIC_CREDENTIAL_HEADERS: [&str; 2] = ["authorization", "x-api-key"];

pub const VERTEX_NEVER_FORWARDED: [&str; 6] = [
    "api-key",
    "content-length",
    "host",
    "ocp-apim-subscription-key",
    "x-api-key",
    "x-litellm-api-key",
];

pub const ANTHROPIC_NEVER_FORWARDED: [&str; 7] = [
    "accept-encoding",
    "api-key",
    "content-length",
    "host",
    "ocp-apim-subscription-key",
    "x-goog-api-key",
    "x-litellm-api-key",
];

pub const BEDROCK_NEVER_FORWARDED: [&str; 9] = [
    "accept-encoding",
    "api-key",
    "authorization",
    "content-length",
    "host",
    "ocp-apim-subscription-key",
    "x-api-key",
    "x-goog-api-key",
    "x-litellm-api-key",
];

pub const MAPPED_ROUTE_CALLER_KEY_HEADER: &str = "litellm_user_api_key";

// B760 owns LiteLLM's LITELLM_INTERNAL_CLAIMS; betterlite has no shared list yet, so the copy lives here
pub const LITELLM_INTERNAL_CLAIMS: [&str; 7] = [
    "_litellm_jwt_issuer",
    "_litellm_user_id",
    "_litellm_user_email",
    "_litellm_team_id",
    "_litellm_team_ids",
    "_litellm_org_id",
    "_litellm_end_user_id",
];

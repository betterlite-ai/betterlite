pub(crate) const CREDENTIAL_HEADER_NAMES: &[&str] = &[
    "authorization",
    "api-key",
    "x-api-key",
    "x-goog-api-key",
    "ocp-apim-subscription-key",
    "x-litellm-api-key",
    "cookie",
    "proxy-authorization",
];
pub(crate) const REDACTED_VALUE: &str = "***REDACTED***";

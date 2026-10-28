//! Pinned deny lists: sentry-sdk 2.21.0's `DEFAULT_DENYLIST`/`DEFAULT_PII_DENYLIST`
//! plus `LiteLLM`'s own `SENTRY_DENYLIST`/`SENTRY_PII_DENYLIST`, combined the way
//! `sentry_scrubbing.py` combines them into `SECRET_FIELD_NAMES`/`PII_FIELD_NAMES`.

pub(super) const KEY_PREFIX: &str = "sk-";
pub(super) const MINIMUM_CUSTOM_KEY_LENGTH: usize = 16;
pub(super) const LENGTH_OF_LITELLM_GENERATED_KEY: usize = 16;

const DEFAULT_DENYLIST: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "api_key",
    "apikey",
    "auth",
    "credentials",
    "mysql_pwd",
    "privatekey",
    "private_key",
    "token",
    "session",
    "csrftoken",
    "sessionid",
    "x_csrftoken",
    "x_forwarded_for",
    "set_cookie",
    "cookie",
    "authorization",
    "x_api_key",
    "aiohttp_session",
    "connect.sid",
    "csrf_token",
    "csrf",
    "_csrf",
    "_csrf_token",
    "PHPSESSID",
    "_session",
    "symfony",
    "user_session",
    "_xsrf",
    "XSRF-TOKEN",
];

const DEFAULT_PII_DENYLIST: &[&str] =
    &["x_forwarded_for", "x_real_ip", "ip_address", "remote_addr"];

const SENTRY_DENYLIST: &[&str] = &[
    "api_key",
    "token",
    "key",
    "secret",
    "password",
    "auth",
    "credential",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "AZURE_API_KEY",
    "COHERE_API_KEY",
    "REPLICATE_API_KEY",
    "HUGGINGFACE_API_KEY",
    "TOGETHERAI_API_KEY",
    "CLOUDFLARE_API_KEY",
    "BASETEN_KEY",
    "OPENROUTER_KEY",
    "COMETAPI_KEY",
    "DATAROBOT_API_TOKEN",
    "FIREWORKS_API_KEY",
    "FIREWORKS_AI_API_KEY",
    "FIREWORKSAI_API_KEY",
    "OVHCLOUD_API_KEY",
    "CLARIFAI_API_KEY",
    "database_url",
    "redis_url",
    "connection_string",
    "master_key",
    "LITELLM_MASTER_KEY",
    "auth_token",
    "jwt_token",
    "private_key",
    "authorization",
    "api-key",
    "x-api-key",
    "x-goog-api-key",
    "ocp-apim-subscription-key",
    "x-litellm-api-key",
    "x-mcp-auth",
    "cookie",
    "set-cookie",
    "SLACK_WEBHOOK_URL",
    "ALERTING_WEBHOOK_URL",
    "webhook_url",
    "LANGFUSE_SECRET_KEY",
    "SMTP_PASSWORD",
    "SMTP_USERNAME",
    "email_password",
    "aws_access_key",
    "aws_secret_key",
    "gcp_credentials",
    "azure_credentials",
    "HCP_VAULT_TOKEN",
    "CIRCLE_OIDC_TOKEN",
    "proxy_url",
    "proxy_key",
    "environment_variables",
];

const SENTRY_PII_DENYLIST: &[&str] = &[
    "user_id",
    "user_email",
    "end_user_id",
    "user_api_key_hash",
    "user_api_key_user_id",
    "user_api_key_user_email",
    "user_api_key_end_user_id",
    "email",
    "phone",
    "address",
    "ip_address",
    "SMTP_SENDER_EMAIL",
    "TEST_EMAIL_ADDRESS",
];

pub(super) fn secret_field_names() -> Vec<&'static str> {
    DEFAULT_DENYLIST
        .iter()
        .chain(SENTRY_DENYLIST)
        .copied()
        .collect()
}

pub(super) fn pii_field_names() -> Vec<&'static str> {
    DEFAULT_PII_DENYLIST
        .iter()
        .chain(SENTRY_PII_DENYLIST)
        .copied()
        .collect()
}

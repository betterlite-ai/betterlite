use std::collections::BTreeSet;

const SUPPORTED_CALLBACK_FIELDS: [&str; 33] = [
    "langfuse_public_key",
    "langfuse_secret",
    "langfuse_secret_key",
    "langfuse_host",
    "langfuse_environment",
    "langfuse_prompt_version",
    "langsmith_api_key",
    "langsmith_project",
    "langsmith_base_url",
    "langsmith_sampling_rate",
    "langsmith_tenant_id",
    "humanloop_api_key",
    "arize_api_key",
    "arize_space_key",
    "arize_space_id",
    "arize_success_sampling_rate",
    "arize_error_sampling_rate",
    "posthog_api_key",
    "posthog_host",
    "braintrust_api_key",
    "braintrust_project",
    "braintrust_host",
    "slack_webhook_url",
    "lunary_public_key",
    "dd_api_key",
    "dd_site",
    "dd_agent_host",
    "dd_agent_port",
    "newrelic_api_key",
    "newrelic_region",
    "signoz_ingestion_endpoint",
    "signoz_ingestion_key",
    "turn_off_message_logging",
];

const REQUEST_BLOCKED_CALLBACK_FIELDS: [&str; 10] = [
    "gcs_bucket_name",
    "gcs_path_service_account",
    "dd_api_key",
    "dd_site",
    "dd_agent_host",
    "dd_agent_port",
    "newrelic_api_key",
    "newrelic_region",
    "signoz_ingestion_endpoint",
    "signoz_ingestion_key",
];

// Prompt version and sampling rate describe the logged request without choosing its destination
const SAFE_CLIENT_CALLBACK_FIELDS: [&str; 2] =
    ["langfuse_prompt_version", "langsmith_sampling_rate"];

// The phoenix project fields stay allowed: the proxy reads them only from trusted key and team metadata
const EXTRA_BANNED_FIELDS: [&str; 4] = [
    "posthog_api_url",
    "user_api_key_auth_metadata",
    "wandb_api_key",
    "weave_project_id",
];

#[must_use]
pub fn banned_fields() -> BTreeSet<&'static str> {
    let mut banned: BTreeSet<&'static str> = SUPPORTED_CALLBACK_FIELDS
        .into_iter()
        .filter(|field| !SAFE_CLIENT_CALLBACK_FIELDS.contains(field))
        .collect();
    banned.extend(REQUEST_BLOCKED_CALLBACK_FIELDS);
    banned.extend(EXTRA_BANNED_FIELDS);
    banned
}

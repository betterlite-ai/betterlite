pub mod changes;
pub mod decoder;
pub mod error;
pub mod fields;
pub mod key;
pub mod team;
mod tests;

// LiteLLM folds these into team and key metadata, so a change to one is attributed to its own name.
const FOLDED_FIELDS: [&str; 28] = [
    "model_rpm_limit",
    "model_tpm_limit",
    "model_itpm_limit",
    "model_otpm_limit",
    "default_estimated_output_tokens",
    "default_estimated_output_tokens_per_model",
    "mcp_rpm_limit",
    "tag_rpm_limit",
    "rpm_limit_type",
    "tpm_limit_type",
    "enforced_params",
    "temp_budget_increase",
    "temp_budget_expiry",
    "allowed_vector_store_indexes",
    "enforced_batch_output_expires_after",
    "enforced_file_expires_after",
    "throttle_on_budget_exceeded",
    "enable_prompt_caching",
    "end_user_budget_id",
    "disable_global_guardrails",
    "guardrails",
    "policies",
    "tags",
    "team_member_key_duration",
    "prompts",
    "logging",
    "secret_manager_settings",
    "allowed_passthrough_routes",
];

const SYSTEM_MANAGED_KEYS: [&str; 1] = ["team_member_budget_id"];
const TEAM_NOT_COLUMNS: [&str; 2] = ["team_id", "metadata"];
const KEY_REQUEST_IDENTITY: [&str; 3] = ["key", "token", "metadata"];
const KEY_BUDGET_FIELDS: [&str; 4] = [
    "max_budget",
    "budget_duration",
    "soft_budget",
    "budget_limits",
];

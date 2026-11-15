use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "betterlite deviation log",
    description = "Where one task's code deliberately differs from the pinned LiteLLM, kept in docs/deviations/<task id>.yaml. The worker writes it, the dispatcher checks its shape and the reviewer reads it."
)]
pub(crate) struct DeviationLog {
    #[schemars(regex(pattern = "^B([0-9]{3}|[1-9][0-9]{3,5})$"))]
    pub(crate) task: String,
    pub(crate) deviations: Vec<LoggedDeviation>,
    #[serde(default)]
    #[schemars(
        description = "LiteLLM bugs the task copies on purpose because a working setup relies on them: what LiteLLM gets wrong, why a setup relies on it, and the https link to the LiteLLM line."
    )]
    pub(crate) kept_bugs: Vec<LoggedDeviation>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoggedDeviation {
    pub(crate) what: String,
    pub(crate) why: String,
    #[schemars(description = "https link to LiteLLM's behavior, or an empty string.")]
    pub(crate) link: String,
}

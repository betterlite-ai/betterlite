use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "betterlite review",
    description = "A reviewer's final answer about one task branch. Every finding cites a rule from .development/rules.yaml."
)]
pub(crate) struct Review {
    #[schemars(description = "The task id, for example B004.")]
    pub(crate) task: String,
    pub(crate) verdict: ReviewVerdict,
    pub(crate) findings: Vec<Finding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ReviewVerdict {
    Approve,
    Changes,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Finding {
    #[schemars(
        description = "A rule id from .development/rules.yaml whose reviewed flag is true."
    )]
    pub(crate) rule: String,
    #[schemars(
        description = "path:line of the code the finding is about, path relative to the repository root."
    )]
    pub(crate) location: String,
    #[schemars(description = "What breaks the rule, in one or two sentences.")]
    pub(crate) problem: String,
    #[schemars(description = "The change that fixes it.")]
    pub(crate) fix: String,
}

use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "betterlite rules",
    description = "The code rules every role works by. Workers follow them, reviewers cite them by id, checks enforce what a tool can."
)]
pub(crate) struct Rules {
    pub(crate) rules: Vec<Rule>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rule {
    #[schemars(regex(pattern = "^[a-z][a-z0-9-]*$"))]
    pub(crate) id: String,
    #[schemars(length(min = 1), description = "The rule in a few words.")]
    pub(crate) title: String,
    #[schemars(
        length(min = 1),
        description = "What the code does, stated as an instruction."
    )]
    pub(crate) statement: String,
    #[schemars(length(min = 1), description = "Why the rule exists.")]
    pub(crate) why: String,
    #[schemars(
        description = "Ids of checks in checks.yaml that fail when the rule is broken. Empty when only a review can tell."
    )]
    pub(crate) enforced_by: Vec<String>,
    #[schemars(description = "True when the reviewer reads every change for this rule.")]
    pub(crate) reviewed: bool,
    pub(crate) example: Example,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Example {
    #[schemars(length(min = 1), description = "Code that breaks the rule.")]
    pub(crate) instead_of: String,
    #[schemars(length(min = 1), description = "The same intent written by the rule.")]
    pub(crate) write: String,
}

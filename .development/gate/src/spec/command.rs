use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    description = "A shell command run with bash -c from the repository root. Exit 0 passes, anything else fails."
)]
pub(crate) struct Command {
    #[schemars(regex(pattern = "^[a-z][a-z0-9-]*$"))]
    pub(crate) id: String,
    #[schemars(
        length(min = 1),
        description = "What a failure means, in one sentence."
    )]
    pub(crate) why: String,
    #[schemars(length(min = 1))]
    pub(crate) run: String,
    #[schemars(
        description = "Path from the repository root. When it does not exist the check is skipped."
    )]
    pub(crate) when_exists: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Run only after every other check passed. A gate that is already red skips it, so slow checks such as mutants never run on unfinished work."
    )]
    pub(crate) after_green: bool,
}

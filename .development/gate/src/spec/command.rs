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
    #[serde(default)]
    #[schemars(
        description = "Run while holding a lock in the git common directory, so gates in other worktrees of this clone wait their turn instead of running it at the same time. For tools such as cargo deny that lock a cache in the home directory and give up after a short wait."
    )]
    pub(crate) one_at_a_time: bool,
}

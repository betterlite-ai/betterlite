use schemars::JsonSchema;
use serde::Deserialize;

pub(crate) const PLACEHOLDERS: [&str; 9] = [
    "model",
    "effort",
    "schema_text",
    "worktree",
    "schema",
    "output",
    "instructions",
    "git_common_dir",
    "home",
];

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "betterlite agents",
    description = "Which CLI and model plays each role, how a run is started, and the limits the dispatcher enforces."
)]
pub(crate) struct Agents {
    #[schemars(
        length(min = 1),
        description = "Directory for task worktrees, relative to the repository root."
    )]
    pub(crate) worktrees: String,
    pub(crate) runners: Vec<Runner>,
    pub(crate) roles: Roles,
    pub(crate) policy: Policy,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    description = "How to start one agent run. The prompt goes to stdin. Arguments may use {model}, {effort}, {worktree}, {schema}, {schema_text}, {output}, {instructions}, {git_common_dir} and {home}."
)]
pub(crate) struct Runner {
    #[schemars(regex(pattern = "^[a-z][a-z0-9-]*$"))]
    pub(crate) id: String,
    pub(crate) vendor: Vendor,
    #[schemars(length(min = 1))]
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) output: OutputMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Vendor {
    Openai,
    Anthropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[schemars(
    description = "file: the runner writes its final JSON answer to {output}. claude-json: stdout is Claude's JSON result and the answer is its structured_output."
)]
pub(crate) enum OutputMode {
    File,
    ClaudeJson,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Roles {
    pub(crate) worker: Role,
    pub(crate) reviewer: Role,
    pub(crate) planner: Role,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Role {
    #[schemars(
        length(min = 1),
        description = "The role's rules file, relative to .development/."
    )]
    pub(crate) instructions: String,
    #[schemars(
        length(min = 1),
        description = "Rungs tried in order. The dispatcher climbs to the next rung when a rung's attempts run out."
    )]
    pub(crate) ladder: Vec<Rung>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rung {
    #[schemars(regex(pattern = "^[a-z][a-z0-9-]*$"))]
    pub(crate) runner: String,
    #[schemars(length(min = 1))]
    pub(crate) model: String,
    #[schemars(length(min = 1))]
    pub(crate) effort: String,
    #[schemars(range(min = 1))]
    pub(crate) attempts: u32,
    #[schemars(range(min = 1))]
    pub(crate) timeout_minutes: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Policy {
    #[schemars(
        description = "The reviewer always runs on a different vendor than the worker whose change it reads."
    )]
    pub(crate) review_by_other_vendor: bool,
    #[schemars(description = "Review rounds before a task goes back to the architect.")]
    pub(crate) max_review_rounds: u32,
    #[schemars(
        description = "Times the Stop hook sends an agent back before it lets the run end red."
    )]
    pub(crate) max_hook_blocks: u32,
}

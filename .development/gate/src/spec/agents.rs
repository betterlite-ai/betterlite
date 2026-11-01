use schemars::JsonSchema;
use serde::Deserialize;

pub(crate) const PLACEHOLDERS: [&str; 12] = [
    "model",
    "effort",
    "schema_text",
    "writable_roots",
    "worktree",
    "schema",
    "output",
    "instructions",
    "git_common_dir",
    "home",
    "sandbox_settings",
    "session",
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
    pub(crate) vendors: Vec<Vendor>,
    pub(crate) runners: Vec<Runner>,
    pub(crate) roles: Roles,
    pub(crate) policy: Policy,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    description = "An account that runs agents. Each runner names one, so a new CLI is one vendor and its runners."
)]
pub(crate) struct Vendor {
    #[schemars(regex(pattern = "^[a-z][a-z0-9-]*$"))]
    pub(crate) id: String,
    #[schemars(
        description = "Text the vendor's CLI prints when it can't run for a while: out of usage, logged out or offline. A run that ends without an answer and prints one in its last lines marks the vendor, and every ladder skips it for limit_retry_minutes."
    )]
    pub(crate) limit_markers: Vec<String>,
    #[schemars(
        range(min = 1),
        description = "Runs at once on this vendor. Without it the policy's max_parallel applies."
    )]
    pub(crate) max_parallel: Option<u32>,
    #[schemars(
        range(min = 1),
        description = "Worker runs at once on this vendor, counted inside its max_parallel. Without it workers may take every seat."
    )]
    pub(crate) max_writers: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    description = "How to start one agent run. The prompt goes to stdin. Arguments may use {model}, {effort}, {worktree}, {schema}, {schema_text}, {output}, {instructions}, {git_common_dir}, {home}, {writable_roots} (the policy's writable folders as a TOML array), {sandbox_settings} (a Claude settings file that sandboxes Bash to the worktree and those folders) and, in resume only, {session}."
)]
pub(crate) struct Runner {
    #[schemars(regex(pattern = "^[a-z][a-z0-9-]*$"))]
    pub(crate) id: String,
    #[schemars(
        regex(pattern = "^[a-z][a-z0-9-]*$"),
        description = "The id of the vendor whose account runs this CLI."
    )]
    pub(crate) vendor: String,
    #[schemars(length(min = 1))]
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    #[schemars(
        description = "Arguments for a fix turn that goes on in the session of the worker run before it, used instead of args when that run had this runner and model and reported its session. {session} expands to the session id. Without them every run starts a new session."
    )]
    pub(crate) resume: Option<Vec<String>>,
    pub(crate) output: OutputMode,
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
    #[schemars(description = "Reviews tasks whose review_level is security.")]
    pub(crate) security_reviewer: Role,
    #[schemars(
        description = "Reads a standard task that cites LiteLLM once the reviewer approves it, for LiteLLM parity, tests that can fail and bounded inputs. Later review rounds of that task stay with it."
    )]
    pub(crate) parity_reviewer: Role,
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
        description = "Rungs tried in order. A worker climbs to the next rung when a rung's attempts run out. While a worker rung's vendor is at its usage limit, its tries go to the first later rung on another vendor and the rung keeps its own attempts; with no such rung the dispatch stops and keeps its progress. A reviewer skips a rung whose vendor is at its usage limit."
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
        description = "The reviewer runs on a different vendor than the worker whose change it reads. When every such reviewer is at its usage limit, one on the worker's vendor runs."
    )]
    pub(crate) review_by_other_vendor: bool,
    #[schemars(
        range(min = 1),
        description = "Fresh worker starts across the whole ladder, tries lent to a later rung included. Fix turns and runs that hit a usage limit don't count. cargo gate dispatch --grant adds starts on the last rung once these are used."
    )]
    pub(crate) max_worker_attempts: u32,
    #[schemars(
        range(min = 1),
        description = "Minutes a vendor stays skipped after one of its runs hit the usage limit. The next run after that tries it again."
    )]
    pub(crate) limit_retry_minutes: u32,
    #[schemars(
        range(min = 1),
        description = "Runs at once on each vendor that sets no max_parallel of its own. A run waits for a free seat."
    )]
    pub(crate) max_parallel: u32,
    #[schemars(
        description = "When every seat of a worker rung's vendor is taken, the try runs on the first later rung of another vendor that has a free seat. It still counts as a try on its own rung."
    )]
    pub(crate) overflow: bool,
    #[schemars(
        description = "Fix turns after one worker start or one review round: runs that go on from a red gate, a rejected report or an edit under .development. They don't count as worker attempts. When they run out, the next run is a fresh start that does."
    )]
    pub(crate) max_fix_turns: u32,
    #[schemars(
        description = "Review rounds before a task goes back to the architect. The worker fixes the findings on the same rung, without a new worker attempt. A dispatch after the rounds ran out gets one more round."
    )]
    pub(crate) max_review_rounds: u32,
    #[schemars(
        description = "Blocked reports in a row before a task goes back to the architect instead of climbing to the next rung."
    )]
    pub(crate) max_blocked_reports: u32,
    #[schemars(
        description = "Times the Stop hook sends an agent back before it lets the run end red."
    )]
    pub(crate) max_hook_blocks: u32,
    #[schemars(
        description = "Folders a worker may write besides its worktree, like the repository's .git folder and tool caches. {git_common_dir} and {home} expand."
    )]
    pub(crate) writable: Vec<String>,
}

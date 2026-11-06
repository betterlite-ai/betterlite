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
        description = "Text the CLI prints when unavailable. Quota waits for the reported reset, or limit_retry_minutes when absent; network errors wait one minute; authentication errors require clearing the marker after repair. An expired marker permits one shared probe."
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
    #[schemars(
        length(min = 1),
        description = "A file of NAME=VALUE lines that the run gets as environment variables, so a key stays out of the repository. {home} expands. Blank lines and lines that start with # are skipped, and a value in single quotes loses them."
    )]
    pub(crate) env_file: Option<String>,
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
        description = "Rungs tried in order. A worker rung's level is its place among its own vendor's rungs. After a failed start the worker stays on its rung while it has attempts left, then moves to its vendor's next rung, then to the first open rung of another vendor at the same level or above. A start that fails the same way as the start before it (the same red check, or the same timeout, silent run or rejected report) goes to that other vendor first. While a worker rung's vendor is at its usage limit, its runs go to that other vendor and the rung keeps its own attempts; with no such rung the dispatch stops and keeps its progress. A reviewer skips a rung whose vendor is at its usage limit."
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
    #[schemars(
        length(min = 1),
        description = "The only tasks this worker rung runs for. Every other task's ladder leaves the rung out, so the places their saved progress points at stay the same."
    )]
    pub(crate) tasks: Option<Vec<String>>,
}

impl Role {
    pub(crate) fn keep_rungs_for(&mut self, task: &str) {
        self.ladder.retain(|rung| {
            rung.tasks
                .as_ref()
                .is_none_or(|tasks| tasks.iter().any(|id| id == task))
        });
    }
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
        description = "Fresh worker starts across the whole ladder, runs on another vendor's rung included. Fix turns and runs that hit a usage limit don't count. cargo gate dispatch --grant adds starts on the last rung once these are used."
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
        description = "When every worker seat of a rung's vendor is taken, the start runs on another vendor's open rung at the same level that has a free seat, and counts there."
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
        description = "Blocked reports in a row before a task goes back to the architect. A report before that resumes the worker's session on the same rung, with a note that unfinished work is not a blocker."
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

use schemars::JsonSchema;
use serde::Deserialize;

use crate::spec::command::Command;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "betterlite task",
    description = "One unit of work for one worker: what to build, how it is proven done, and the commands that prove it."
)]
pub(crate) struct Task {
    #[schemars(
        regex(pattern = "^B[0-9]{3}$"),
        description = "Same id as the task's backlog entry."
    )]
    pub(crate) id: String,
    #[schemars(length(min = 1), description = "One line, plain words.")]
    pub(crate) title: String,
    #[schemars(
        length(min = 1),
        description = "What to build and why, in prose. A worker with no other context must be able to start from it."
    )]
    pub(crate) description: String,
    pub(crate) scope: Scope,
    #[schemars(
        description = "Pages to read before starting. LiteLLM links point at the pinned oracle commit."
    )]
    pub(crate) references: Vec<Reference>,
    #[schemars(
        length(min = 1),
        description = "Definition of done. Every item names the checks that prove it."
    )]
    pub(crate) dod: Vec<Dod>,
    pub(crate) checks: TaskChecks,
    #[schemars(
        description = "Oracle case files (paths from the repository root) whose LiteLLM and betterlite outputs must match. A non-empty list adds the oracle check."
    )]
    pub(crate) oracle_cases: Vec<String>,
    #[schemars(description = "Places where betterlite deliberately differs from LiteLLM.")]
    pub(crate) deviations: Vec<Deviation>,
    #[schemars(description = "Work that belongs to other tasks. The worker leaves it alone.")]
    pub(crate) out_of_scope: Vec<String>,
    #[schemars(description = "Known traps, each as one plain sentence.")]
    pub(crate) pitfalls: Vec<String>,
    pub(crate) budget: Budget,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(description = "Files the task may change.")]
pub(crate) struct Scope {
    #[schemars(
        length(min = 1),
        description = "Glob patterns from the repository root. `*` stays inside one directory, `**` crosses directories."
    )]
    pub(crate) paths: Vec<String>,
    #[schemars(
        description = "True only for a task allowed to change [workspace.lints] in the root Cargo.toml."
    )]
    pub(crate) lint_tables: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reference {
    #[schemars(length(min = 1))]
    pub(crate) url: String,
    #[schemars(length(min = 1), description = "What the worker takes from this page.")]
    pub(crate) why: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Dod {
    #[schemars(length(min = 1), description = "One observable outcome.")]
    pub(crate) text: String,
    #[schemars(
        length(min = 1),
        inner(regex(pattern = "^[a-z][a-z0-9-]*$")),
        description = "Ids of checks this task runs (base, optional or its own) that fail when the outcome is missing."
    )]
    pub(crate) proof: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    description = "Checks on top of the base set in checks.yaml. Every one of them must pass."
)]
pub(crate) struct TaskChecks {
    #[schemars(
        inner(regex(pattern = "^[a-z][a-z0-9-]*$")),
        description = "Ids from the optional list in checks.yaml."
    )]
    pub(crate) optional: Vec<String>,
    #[schemars(description = "Commands only this task runs.")]
    pub(crate) commands: Vec<Command>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Deviation {
    #[schemars(length(min = 1), description = "What betterlite does differently.")]
    pub(crate) what: String,
    #[schemars(length(min = 1), description = "Why the difference is deliberate.")]
    pub(crate) why: String,
    pub(crate) link: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Budget {
    #[schemars(
        range(min = 1),
        description = "Added plus deleted lines against the base, without the paths in settings.budget_excludes."
    )]
    pub(crate) max_changed_lines: usize,
    #[schemars(description = "New #[expect(...)] attributes the task may add.")]
    pub(crate) max_new_expects: usize,
}

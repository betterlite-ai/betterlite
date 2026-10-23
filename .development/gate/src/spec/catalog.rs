use schemars::JsonSchema;
use serde::Deserialize;
use strum::IntoStaticStr;

use crate::spec::command::Command;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "betterlite checks",
    description = "The tools every check needs, shared settings, the base checks every task runs, and the optional checks a task can add."
)]
pub(crate) struct Catalog {
    #[schemars(
        description = "CLI tools at exact versions. The tools builtin fails when one differs."
    )]
    pub(crate) tools: Vec<Tool>,
    pub(crate) settings: Settings,
    pub(crate) base: BaseChecks,
    pub(crate) optional: Vec<Command>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tool {
    #[schemars(length(min = 1))]
    pub(crate) name: String,
    #[schemars(length(min = 1))]
    pub(crate) version_command: String,
    #[schemars(
        length(min = 1),
        description = "Text the first line of the version output must contain."
    )]
    pub(crate) expect: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    #[schemars(description = "Globs every task may change, on top of its own scope.")]
    pub(crate) always_in_scope: Vec<String>,
    #[schemars(description = "Globs left out of the changed-line budget.")]
    pub(crate) budget_excludes: Vec<String>,
    pub(crate) file_size: FileSize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileSize {
    #[schemars(description = "Directories whose .rs files are measured.")]
    pub(crate) roots: Vec<String>,
    pub(crate) src_max_lines: usize,
    #[schemars(description = "Limit for tests.rs files and files under a tests/ directory.")]
    pub(crate) test_max_lines: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BaseChecks {
    pub(crate) builtin: Vec<Builtin>,
    pub(crate) commands: Vec<Command>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Builtin {
    pub(crate) id: BuiltinId,
    #[schemars(length(min = 1))]
    pub(crate) why: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema, IntoStaticStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub(crate) enum BuiltinId {
    TaskFile,
    Tools,
    Scope,
    LintTable,
    DiffBudget,
    ExpectBudget,
    FileSize,
    Crates,
}

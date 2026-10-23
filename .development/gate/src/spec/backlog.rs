use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "betterlite backlog",
    description = "Every task, its status and its dependencies. Only the architect edits it, on main."
)]
pub(crate) struct Backlog {
    pub(crate) tasks: Vec<BacklogEntry>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BacklogEntry {
    #[schemars(regex(pattern = "^B[0-9]{3}$"))]
    pub(crate) id: String,
    #[schemars(length(min = 1))]
    pub(crate) milestone: String,
    #[schemars(length(min = 1))]
    pub(crate) title: String,
    pub(crate) status: Status,
    #[schemars(inner(regex(pattern = "^B[0-9]{3}$")))]
    pub(crate) depends_on: Vec<String>,
    #[schemars(
        description = "Task file under .development/, for example tasks/B004-openai-chat.yaml. Null until the task is written."
    )]
    pub(crate) file: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Status {
    Todo,
    InProgress,
    Done,
    Blocked,
}

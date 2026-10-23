use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "betterlite backlog",
    description = "Every task, its status, its dependencies and the LiteLLM paths it covers. The architect lands every change to it."
)]
pub(crate) struct Backlog {
    pub(crate) tasks: Vec<BacklogEntry>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BacklogEntry {
    #[schemars(regex(pattern = "^B([0-9]{3}|[1-9][0-9]{3,5})$"))]
    pub(crate) id: String,
    #[schemars(length(min = 1))]
    pub(crate) milestone: String,
    #[schemars(length(min = 1))]
    pub(crate) title: String,
    pub(crate) status: Status,
    #[schemars(inner(regex(pattern = "^B([0-9]{3}|[1-9][0-9]{3,5})$")))]
    pub(crate) depends_on: Vec<String>,
    #[schemars(
        description = "Task file under .development/, for example tasks/B004-openai-chat.yaml. Null until the task is written."
    )]
    pub(crate) file: Option<String>,
    #[schemars(
        description = "Globs over the LiteLLM paths in litellm-inventory.txt whose behavior this entry reimplements. Empty for work betterlite needs that LiteLLM has no file for."
    )]
    pub(crate) covers: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Status {
    Todo,
    InProgress,
    Done,
    Blocked,
}

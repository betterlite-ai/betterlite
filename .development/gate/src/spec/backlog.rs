use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug)]
pub(crate) struct Backlog {
    pub(crate) tasks: Vec<BacklogEntry>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "betterlite backlog entry",
    description = "One task in .development/backlog/<id>.yaml: its status, its dependencies and the LiteLLM paths it covers. The architect lands every change to it."
)]
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
    #[schemars(
        regex(pattern = "^B([0-9]{3}|[1-9][0-9]{3,5})$"),
        description = "The umbrella entry this part belongs to, or null for an umbrella or a standalone entry."
    )]
    pub(crate) parent: Option<String>,
    #[schemars(
        regex(pattern = "^B([0-9]{3}|[1-9][0-9]{3,5})$"),
        description = "The entry this one was split off from when that is known, otherwise null."
    )]
    pub(crate) split_from: Option<String>,
    #[schemars(description = "How a done entry ended. Null until status is done.")]
    pub(crate) resolution: Option<Resolution>,
    #[schemars(
        regex(pattern = "^B([0-9]{3}|[1-9][0-9]{3,5})$"),
        description = "The entry that replaces a duplicate or superseded one, otherwise null."
    )]
    pub(crate) superseded_by: Option<String>,
    #[schemars(
        description = "Why the entry is blocked, in one line. Null unless status is blocked."
    )]
    pub(crate) blocked_reason: Option<String>,
    #[schemars(
        description = "The pull request that merged or is merging the entry's work, otherwise null."
    )]
    pub(crate) pr: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Status {
    Todo,
    InProgress,
    Done,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Resolution {
    Merged,
    Obsolete,
    Duplicate,
    Superseded,
}

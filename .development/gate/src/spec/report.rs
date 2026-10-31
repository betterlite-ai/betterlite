use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "betterlite worker report",
    description = "A worker's final answer. Every field is required; use an empty string or list when there is nothing to say."
)]
pub(crate) struct Report {
    #[schemars(description = "The task id, for example B004.")]
    pub(crate) task: String,
    pub(crate) status: ReportStatus,
    #[schemars(
        regex(pattern = ".{40,}"),
        description = "What changed, in two to four plain sentences, at least 40 characters."
    )]
    pub(crate) summary: String,
    #[schemars(
        description = "Empty when done. When blocked: what is needed, where, and the evidence."
    )]
    pub(crate) blocker: String,
    #[schemars(
        description = "One entry per behavior a user can observe in the LiteLLM code the task's references own. Behavior another task owns goes in notes with that task's id."
    )]
    pub(crate) owned: Vec<OwnedBehavior>,
    #[schemars(
        description = "One entry per problem solved by choosing an existing solution or deliberately writing one."
    )]
    pub(crate) prior_art: Vec<PriorArt>,
    pub(crate) deviations: Vec<ReportDeviation>,
    #[schemars(description = "Facts the architect needs and nothing else.")]
    pub(crate) notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ReportStatus {
    Done,
    Blocked,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnedBehavior {
    #[schemars(
        description = "The behavior, for example: OpenAI-Beta: assistants=v2 is added when the caller didn't send it."
    )]
    pub(crate) behavior: String,
    #[schemars(description = "https link to the LiteLLM line that does it, at the pinned commit.")]
    pub(crate) litellm: String,
    #[schemars(description = "Where betterlite does it, as path:line.")]
    pub(crate) code: String,
    #[schemars(description = "The test name or oracle case that proves it.")]
    pub(crate) test: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PriorArt {
    #[schemars(
        description = "The problem, for example: parse server-sent events split across network reads."
    )]
    pub(crate) need: String,
    #[schemars(
        description = "At least two options that were compared, exactly one of them chosen."
    )]
    pub(crate) options: Vec<PriorOption>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PriorOption {
    #[schemars(description = "A crate name exactly as in Cargo.toml, a std API, or hand-written.")]
    pub(crate) name: String,
    #[schemars(description = "https link to the crate, docs or code that was read.")]
    pub(crate) link: String,
    pub(crate) verdict: OptionVerdict,
    #[schemars(description = "Which requirement it meets or misses.")]
    pub(crate) why: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum OptionVerdict {
    Chosen,
    Rejected,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReportDeviation {
    pub(crate) what: String,
    pub(crate) why: String,
    #[schemars(description = "https link to LiteLLM's behavior, or an empty string.")]
    pub(crate) link: String,
}

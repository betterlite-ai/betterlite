use betterlite_openai::openai_base_config::Object;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) token_alias_overwrite: TokenAliasCase,
    pub(crate) self_deployed_exclusions: ExclusionsCase,
    pub(crate) gpt_oss_singular_tool: GptOssCase,
    pub(crate) missing_reply_defaults: ReplyCase,
    pub(crate) initial_missing_role: StreamCase,
    pub(crate) initial_terminal_split: StreamCase,
    pub(crate) empty_initial_delta: StreamCase,
}

// both partners move max_completion_tokens onto max_tokens before the inherited mapping runs
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TokenAliasCase {
    pub(crate) ai21_model: String,
    pub(crate) ai21_requested: Object,
    pub(crate) ai21_optional_params: Object,
    pub(crate) llama_model: String,
    pub(crate) llama_requested: Object,
    pub(crate) llama_optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExclusionsCase {
    pub(crate) publisher_model: String,
    pub(crate) deployed_model: String,
    pub(crate) requested: Object,
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GptOssCase {
    pub(crate) model: String,
    pub(crate) funded_model: String,
    pub(crate) requested: Object,
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCase {
    pub(crate) requested_model: String,
    pub(crate) reply: Object,
    pub(crate) named_reply: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamCase {
    pub(crate) chunks: Vec<Object>,
}

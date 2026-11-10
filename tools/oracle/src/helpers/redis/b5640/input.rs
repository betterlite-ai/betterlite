use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    BatchRead(BatchReadCall),
    TtlReply(TtlReplyCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchReadCall {
    pub(crate) namespace: Option<String>,
    #[serde(default)]
    pub(crate) keys: Vec<Option<String>>,
    #[serde(default)]
    pub(crate) replies: Vec<Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TtlReplyCall {
    pub(crate) namespace: Option<String>,
    pub(crate) key: String,
    pub(crate) reply: i64,
}

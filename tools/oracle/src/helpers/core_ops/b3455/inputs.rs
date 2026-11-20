use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) options: Value,
    #[serde(default)]
    pub(crate) file: FileInput,
    #[serde(default)]
    pub(crate) responses: Vec<ReplyInput>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileInput {
    #[serde(default)]
    pub(crate) content_base64: Option<String>,
    #[serde(default)]
    pub(crate) filename: Option<String>,
    #[serde(default)]
    pub(crate) content_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyInput {
    pub(crate) status: u16,
    pub(crate) body: String,
}

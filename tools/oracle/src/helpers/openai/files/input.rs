use std::num::NonZeroUsize;

use betterlite_openai::files::records::CreatePurpose;
use indexmap::IndexMap;
use serde::Deserialize;

use super::fixture::Reply;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Action {
    CreateFile,
    FileContent,
    FileContentStreaming,
    RetrieveFile,
    DeleteFile,
    ListFiles,
    Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Supplied {
    Sync,
    Async,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClientSpec {
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) timeout: Option<u64>,
    pub(super) max_retries: Option<u32>,
    pub(super) organization: Option<String>,
    pub(super) supplied: Option<Supplied>,
    pub(super) is_async: bool,
    #[serde(default)]
    pub(super) missing: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    #[serde(default)]
    pub(super) file_id: String,
    #[serde(default)]
    pub(super) purpose: Option<CreatePurpose>,
    #[serde(default)]
    pub(super) list_purpose: Option<String>,
    #[serde(default)]
    pub(super) filename: Option<String>,
    #[serde(default)]
    pub(super) content_type: Option<String>,
    #[serde(default)]
    pub(super) data: String,
    #[serde(default)]
    pub(super) expires_after: Option<i64>,
    #[serde(default)]
    pub(super) extra_headers: IndexMap<String, String>,
    #[serde(default)]
    pub(super) extra_body: IndexMap<String, String>,
    #[serde(default)]
    pub(super) timeout: Option<u64>,
    #[serde(default)]
    pub(super) chunk_size: Option<NonZeroUsize>,
    #[serde(default)]
    pub(super) read: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) action: Action,
    pub(super) client: ClientSpec,
    #[serde(default)]
    pub(super) request: Request,
    #[serde(default)]
    pub(super) reply: Option<Reply>,
}

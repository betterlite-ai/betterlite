use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(super) supported: bool,
    #[serde(default)]
    pub(super) maps: Vec<MapCall>,
    #[serde(default)]
    pub(super) joins: Vec<JoinCall>,
    #[serde(default)]
    pub(super) urls: Vec<UrlCall>,
    #[serde(default)]
    pub(super) creates: Vec<CreateCall>,
    #[serde(default)]
    pub(super) lists: Vec<ListCall>,
    #[serde(default)]
    pub(super) file_lists: Vec<FileListCall>,
    #[serde(default)]
    pub(super) resources: Vec<ResourceCall>,
    #[serde(default)]
    pub(super) contents: Vec<ContentCall>,
    #[serde(default)]
    pub(super) headers: Vec<HeadersCall>,
    #[serde(default)]
    pub(super) create_replies: Vec<ReplyCall>,
    #[serde(default)]
    pub(super) metadata_replies: Vec<MetadataCall>,
    #[serde(default)]
    pub(super) content_replies: Vec<ReplyCall>,
    #[serde(default)]
    pub(super) errors: Vec<ErrorCall>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Globals {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) openai_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapCall {
    #[serde(default)]
    pub(super) params: Map<String, Value>,
    pub(super) drop_params: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JoinCall {
    pub(super) api_base: String,
    pub(super) path_suffix: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCall {
    pub(super) api_base: Option<String>,
    pub(super) globals: Globals,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCall {
    pub(super) name: String,
    #[serde(default)]
    pub(super) params: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListCall {
    pub(super) api_base: String,
    pub(super) after: Option<String>,
    pub(super) limit: Option<i64>,
    pub(super) order: Option<String>,
    pub(super) extra_query: Option<Map<String, Value>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileListCall {
    pub(super) container_id: String,
    pub(super) api_base: String,
    pub(super) after: Option<String>,
    pub(super) limit: Option<i64>,
    pub(super) order: Option<String>,
    pub(super) extra_query: Option<Map<String, Value>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResourceKind {
    Retrieve,
    Delete,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResourceCall {
    pub(super) kind: ResourceKind,
    pub(super) container_id: String,
    pub(super) api_base: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContentCall {
    pub(super) container_id: String,
    pub(super) file_id: String,
    pub(super) api_base: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeadersCall {
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    pub(super) globals: Globals,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCall {
    pub(super) body: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetadataKind {
    Retrieve,
    List,
    Delete,
    FileList,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetadataCall {
    pub(super) kind: MetadataKind,
    pub(super) body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorCall {
    pub(super) message: String,
    pub(super) status: u16,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
}

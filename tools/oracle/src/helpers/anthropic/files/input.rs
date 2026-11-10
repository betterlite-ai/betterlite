use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    FileUploadMultipart(UploadCase),
    NativeFileObject(FileObjectCase),
    CreatedAtTimezone(CreatedAtCase),
    FileListPagination(ListCase),
    FileDeleteShape(DeleteCase),
    FileContentBytes(ContentCase),
    FileBetaHeader(BetaHeaderCase),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadCase {
    pub(crate) now: i64,
    pub(crate) uploads: Vec<UploadItem>,
    pub(crate) failure: Option<FailureFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FailureFixture {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadItem {
    #[serde(default)]
    pub(crate) missing: bool,
    pub(crate) filename: Option<String>,
    pub(crate) content_type: Option<String>,
    #[serde(default)]
    pub(crate) data: String,
    pub(crate) purpose: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileObjectCase {
    pub(crate) now: i64,
    pub(crate) bodies: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreatedAtCase {
    pub(crate) now: i64,
    pub(crate) values: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListCase {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) params_api_base: Option<String>,
    pub(crate) now: i64,
    pub(crate) purposes: Vec<Option<String>>,
    pub(crate) body: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteCase {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) params_api_base: Option<String>,
    pub(crate) file_ids: Vec<String>,
    pub(crate) bodies: Vec<Value>,
    pub(crate) failure: Option<FailureFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContentCase {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) params_api_base: Option<String>,
    pub(crate) file_ids: Vec<String>,
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) error: ErrorFixture,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BetaHeaderCase {
    pub(crate) scenarios: Vec<BetaScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BetaScenario {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) params_api_base: Option<String>,
    #[serde(default)]
    pub(crate) url_bases: Vec<Option<String>>,
    #[serde(default)]
    pub(crate) caller_headers: Vec<(String, String)>,
    pub(crate) error: ErrorFixture,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorFixture {
    pub(crate) status: u16,
    pub(crate) message: String,
    pub(crate) headers: Vec<(String, String)>,
}

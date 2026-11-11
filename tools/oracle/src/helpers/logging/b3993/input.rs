use betterlite_logging::s3_transport::batch_element::ContentType;
use betterlite_models::decisions::Json;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    pub(crate) settings: Settings,
    pub(crate) credentials: Keys,
    pub(crate) now: u64,
    #[serde(default)]
    pub(crate) element: Option<Element>,
    #[serde(default)]
    pub(crate) object_key: Option<String>,
    pub(crate) responses: Vec<Reply>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Put,
    Get,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the fields mirror the Settings model of the case input one to one"
)]
pub(crate) struct Settings {
    pub(crate) bucket_name: String,
    pub(crate) region_name: String,
    #[serde(default)]
    pub(crate) endpoint_url: Option<String>,
    #[serde(default)]
    pub(crate) use_virtual_hosted_style: bool,
    #[serde(default)]
    pub(crate) server_side_encryption: Option<String>,
    #[serde(default)]
    pub(crate) sse_kms_key_id: Option<String>,
    #[serde(default = "default_drop_on_terminal_error")]
    pub(crate) drop_on_terminal_error: bool,
    #[serde(default)]
    pub(crate) adaptive_concurrency: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Keys {
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: String,
    #[serde(default)]
    pub(crate) session_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Element {
    pub(crate) s3_object_key: String,
    pub(crate) s3_object_download_filename: String,
    #[serde(default)]
    pub(crate) body: Option<String>,
    #[serde(default)]
    pub(crate) content_type: ContentType,
    #[serde(default)]
    pub(crate) payload: Option<Json>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) body: String,
}

const fn default_drop_on_terminal_error() -> bool {
    true
}

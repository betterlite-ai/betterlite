use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};

const DEFAULT_TOKEN_LIFETIME_SECONDS: u64 = 3599;

const fn default_expires_in() -> u64 {
    DEFAULT_TOKEN_LIFETIME_SECONDS
}

#[derive(Deserialize)]
pub struct GraphTokenResponse {
    pub access_token: String,
    #[serde(default = "default_expires_in")]
    pub expires_in: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Activity {
    UploadText,
    DownloadText,
}

#[derive(Serialize)]
pub struct PolicyLocation<'req> {
    #[serde(rename = "@odata.type")]
    pub odata_type: &'static str,
    pub value: &'req str,
}

#[derive(Serialize)]
pub struct ProtectionScopesRequest<'req> {
    pub activities: &'static str,
    pub locations: [PolicyLocation<'req>; 1],
}

#[derive(Serialize)]
pub struct ProcessContentRequest<'req> {
    #[serde(rename = "contentToProcess")]
    pub content_to_process: ContentToProcess<'req>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentToProcess<'req> {
    pub content_entries: [ContentEntry<'req>; 1],
    pub activity_metadata: ActivityMetadata,
    pub device_metadata: Object,
    pub protected_app_metadata: ProtectedAppMetadata<'req>,
    pub integrated_app_metadata: IntegratedAppMetadata<'req>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentEntry<'req> {
    #[serde(rename = "@odata.type")]
    pub odata_type: &'static str,
    pub identifier: String,
    pub content: TextContent<'req>,
    pub name: String,
    pub correlation_id: String,
    pub sequence_number: u32,
    pub is_truncated: bool,
}

#[derive(Serialize)]
pub struct TextContent<'req> {
    #[serde(rename = "@odata.type")]
    pub odata_type: &'static str,
    pub data: &'req str,
}

#[derive(Serialize)]
pub struct ActivityMetadata {
    pub activity: Activity,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtectedAppMetadata<'req> {
    pub name: &'req str,
    pub version: &'static str,
    pub application_location: PolicyLocation<'req>,
}

#[derive(Serialize)]
pub struct IntegratedAppMetadata<'req> {
    pub name: &'req str,
    pub version: &'static str,
}

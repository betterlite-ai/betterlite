use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct UploadResponse {
    #[serde(rename = "jobId")]
    pub job_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Block,
    Modify,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Default, Deserialize)]
pub struct Metadata {
    pub action: Option<Action>,
    pub violations: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct StatusResponse {
    pub status: Option<String>,
    pub content: Option<String>,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizeResult {
    pub action: Action,
    pub content: Option<String>,
    pub violations: Vec<String>,
}

impl From<StatusResponse> for SanitizeResult {
    fn from(polled: StatusResponse) -> Self {
        let metadata = polled.metadata.unwrap_or_default();
        Self {
            action: metadata.action.unwrap_or(Action::Allow),
            content: polled.content,
            violations: metadata.violations.unwrap_or_default(),
        }
    }
}

use betterlite_core::api::fal::video_prepare::VideoHeaders;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusCase {
    pub(crate) status_url: String,
    #[serde(default)]
    pub(crate) headers: VideoHeaders,
    pub(crate) body: String,
    // a missing probe status scripts a transport failure, which LiteLLM treats as transient
    #[serde(default)]
    pub(crate) probe_status: Option<u16>,
    #[serde(default)]
    pub(crate) probe_body: String,
    #[serde(default)]
    pub(crate) provider: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContentCase {
    pub(crate) body: String,
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) response_headers: VideoHeaders,
    // a missing artifact status scripts a transport failure before any artifact answer
    pub(crate) artifact_status: Option<u16>,
    pub(crate) artifact_body: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    CompletedTerminalError { cases: Vec<StatusCase> },
    TransientProbeKeepsCompleted { cases: Vec<StatusCase> },
    DetailLocations { cases: Vec<StatusCase> },
    RecoveredRequestIdentity { cases: Vec<StatusCase> },
    ArtifactWithoutCredentials { cases: Vec<ContentCase> },
    MissingVideoUrl { cases: Vec<ContentCase> },
    UnsupportedRequest,
}

use betterlite_core::api::realtime_contracts::events::{
    AudioTranscriptionUsage, RealtimeResponseTypedDict,
};
use betterlite_models::decisions::Json;
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct CommandsOutput {
    pub(crate) created: Option<Json>,
    pub(crate) steps: Vec<StepOutput>,
    pub(crate) usage: Option<AudioTranscriptionUsage>,
    pub(crate) repeated_usage: Option<AudioTranscriptionUsage>,
}

#[derive(Serialize)]
pub(crate) struct StepOutput {
    pub(crate) commands: Vec<CommandOutput>,
    pub(crate) error: bool,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum CommandOutput {
    Text { value: String },
    Audio { base64: String },
}

#[derive(Serialize)]
pub(crate) struct UrlsOutput {
    pub(crate) urls: Vec<UrlOutput>,
}

#[derive(Serialize)]
pub(crate) struct UrlOutput {
    pub(crate) url: Option<String>,
    pub(crate) error: bool,
}

#[derive(Serialize)]
pub(crate) struct ResponsesOutput {
    pub(crate) created: Option<Json>,
    pub(crate) responses: Vec<ResponseOutput>,
    pub(crate) usage: Option<AudioTranscriptionUsage>,
    pub(crate) repeated_usage: Option<AudioTranscriptionUsage>,
}

#[derive(Serialize)]
pub(crate) struct ResponseOutput {
    pub(crate) result: Option<RealtimeResponseTypedDict>,
    pub(crate) error: Option<ResponseErrorOutput>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ResponseErrorOutput {
    Protocol,
    Invalid,
}

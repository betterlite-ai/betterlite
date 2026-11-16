use betterlite_core::api::realtime_contracts::events::RealtimeResponseTransformInput;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Function {
    Commands {
        project: String,
        #[serde(default)]
        location: Option<String>,
        model: String,
        #[serde(default)]
        session_id: Option<String>,
        steps: Vec<String>,
    },
    Urls {
        project: String,
        #[serde(default)]
        location: Option<String>,
        cases: Vec<UrlCase>,
    },
    Responses(ResponsesCall),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponsesCall {
    pub(crate) project: String,
    #[serde(default)]
    pub(crate) location: Option<String>,
    pub(crate) model: String,
    pub(crate) session_id: String,
    #[serde(default)]
    pub(crate) configure: Option<String>,
    pub(crate) state: RealtimeResponseTransformInput,
    pub(crate) frames: Vec<FrameInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCase {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum FrameInput {
    Text { value: String },
    Bytes { base64: String },
}

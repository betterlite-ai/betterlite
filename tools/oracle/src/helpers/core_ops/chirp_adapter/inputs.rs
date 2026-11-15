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
    Responses {
        project: String,
        #[serde(default)]
        location: Option<String>,
        model: String,
        session_id: String,
        #[serde(default)]
        configure: Option<String>,
        state: RealtimeResponseTransformInput,
        frames: Vec<FrameInput>,
    },
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

use betterlite_models::decisions::Json;
use serde::Serialize;

#[derive(Clone, Serialize)]
pub(super) struct Write {
    pub(super) key: String,
    pub(super) ttl: f64,
}

#[derive(Clone, Serialize)]
pub(super) struct Request {
    pub(super) url: String,
    pub(super) authorization: String,
    pub(super) accept: String,
}

// The proxy exception's answer fields, or a failure the proxy hands to the caller unwrapped.
#[derive(Default, Serialize)]
pub(super) struct Failure {
    pub(super) status: Option<u16>,
    #[serde(rename = "type")]
    pub(super) error_type: Option<String>,
    pub(super) param: Option<String>,
    pub(super) code: Option<String>,
}

#[derive(Serialize)]
pub(super) struct StepAnswer {
    pub(super) value: Json,
    pub(super) error: Option<Failure>,
    pub(super) warnings: Vec<String>,
    pub(super) requests: Vec<Request>,
    pub(super) writes: Vec<Write>,
}

#[derive(Serialize)]
pub(super) struct ScenarioAnswer {
    pub(super) steps: Vec<StepAnswer>,
}

#[derive(Serialize)]
pub(super) struct Answer {
    pub(super) scenarios: Vec<ScenarioAnswer>,
}

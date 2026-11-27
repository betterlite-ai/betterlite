use betterlite_models::decisions::Json;
use serde::Serialize;

#[derive(Clone, Serialize)]
pub(super) struct Write {
    pub(super) key: String,
    pub(super) ttl: f64,
}

#[derive(Serialize)]
pub(super) struct Failure {
    pub(super) status: u16,
    #[serde(rename = "type")]
    pub(super) error_type: String,
    pub(super) param: Option<String>,
    pub(super) code: String,
}

#[derive(Serialize)]
pub(super) struct StepAnswer {
    pub(super) value: Json,
    pub(super) error: Option<Failure>,
    pub(super) requests: Vec<String>,
    pub(super) sleeps: Vec<f64>,
    pub(super) writes: Vec<Write>,
    pub(super) warnings: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct ScenarioAnswer {
    pub(super) settings_error: bool,
    pub(super) steps: Vec<StepAnswer>,
}

#[derive(Serialize)]
pub(super) struct Answer {
    pub(super) scenarios: Vec<ScenarioAnswer>,
}

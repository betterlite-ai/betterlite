use betterlite_models::decisions::Json;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) result: ResultWire,
    #[serde(default)]
    pub(super) dynamic_success: Vec<String>,
    #[serde(default)]
    pub(super) global_success: Vec<LoggerWire>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResultWire {
    pub(super) content: Vec<Json>,
    #[serde(default)]
    pub(super) structured_content: Option<Json>,
    #[serde(default)]
    pub(super) is_error: bool,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoggerWire {
    pub(super) name: String,
    pub(super) behavior: Behavior,
    #[serde(default)]
    pub(super) content: Vec<Json>,
    #[serde(default)]
    // ast-grep-ignore: no-float-money
    pub(super) response_cost: Option<f64>,
    #[serde(default)]
    pub(super) structured_content: Option<Json>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Behavior {
    None,
    EditInPlace,
    Replace,
    ReplaceUnchanged,
    Fail,
    RewriteLive,
    RewriteLiveThenFail,
}

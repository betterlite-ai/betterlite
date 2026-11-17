use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Case {
    pub(super) path: Route,
    pub(super) api_base: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) data: String,
    #[serde(default)]
    pub(super) signed_json_body: Option<String>,
    pub(super) model: String,
    pub(super) messages: Json,
    #[serde(default)]
    pub(super) json_mode: Option<bool>,
    pub(super) fake_stream: bool,
    pub(super) status: u16,
    #[serde(default)]
    pub(super) response_headers: BTreeMap<String, String>,
    // the exact bytes one transport read of the fixture response delivers
    pub(super) body: String,
    #[serde(default)]
    pub(super) failure: Option<Failure>,
    pub(super) consume: Consume,
    // only the fields both languages can reproduce are compared
    #[serde(default)]
    pub(super) compare_message: bool,
    #[serde(default)]
    pub(super) compare_headers: bool,
    #[serde(default)]
    pub(super) provider: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Route {
    InvokeSync,
    InvokeAsync,
    ConverseSync,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Failure {
    Timeout,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Consume {
    None,
    First,
    All,
}

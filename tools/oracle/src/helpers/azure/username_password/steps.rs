use std::collections::BTreeMap;

use serde::Deserialize;

use super::fixture::Reply;

#[derive(Deserialize)]
pub(crate) struct Steps {
    pub(super) steps: Vec<Step>,
}

#[derive(Deserialize)]
pub(super) struct Step {
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) authority_host: Option<String>,
    #[serde(default)]
    pub(super) client_id: String,
    #[serde(default)]
    pub(super) username: String,
    #[serde(default)]
    pub(super) password: String,
    #[serde(default)]
    pub(super) scope: Option<String>,
    #[serde(default)]
    pub(super) replies: Vec<Reply>,
    pub(super) calls: Option<usize>,
    #[serde(default)]
    pub(super) advance: i64,
    #[serde(default)]
    pub(super) observe: Observe,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Observe {
    #[default]
    Token,
    Info,
    Lazy,
}

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

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
    pub(super) tenant_id: String,
    #[serde(default)]
    pub(super) client_id: String,
    #[serde(default)]
    pub(super) client_secret: String,
    #[serde(default)]
    pub(super) scope: Option<String>,
    #[serde(default)]
    pub(super) authority_host: Option<String>,
    #[serde(default)]
    pub(super) replies: Vec<Reply>,
    #[serde(default)]
    pub(super) advance: Option<i64>,
    #[serde(default)]
    pub(super) calls: Vec<Call>,
    #[serde(default)]
    pub(super) status_code: u16,
    #[serde(default)]
    pub(super) message: String,
    #[serde(default)]
    pub(super) headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) request: Option<RequestFixture>,
    #[serde(default)]
    pub(super) response: Option<ResponseFixture>,
    #[serde(default)]
    pub(super) body: Option<Value>,
}

#[derive(Deserialize)]
pub(super) struct Call {
    pub(super) tenant_id: String,
    pub(super) client_id: String,
    pub(super) client_secret: String,
    #[serde(default)]
    pub(super) scope: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RequestFixture {
    pub(super) method: String,
    pub(super) url: String,
}

#[derive(Deserialize)]
pub(super) struct ResponseFixture {
    pub(super) status_code: u16,
}

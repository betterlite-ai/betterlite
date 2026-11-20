use std::collections::BTreeMap;

use betterlite_oracle_helper_api::error::{HelperError, HelperResult};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub(crate) enum Input {
    Policies(Policies),
    Collection(Collection),
    Index(Index),
    Role(Role),
    Flow(Flow),
    Store(Store),
}

#[derive(Debug, Deserialize)]
pub(crate) struct Policies {
    pub(super) vector: BTreeMap<String, Value>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) unique_id: String,
    pub(super) account_id: String,
    pub(super) caller_arn: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Collection {
    pub(super) vector: BTreeMap<String, Value>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) unique_id: String,
    pub(super) account_id: String,
    pub(super) caller_arn: String,
    #[serde(default)]
    pub(super) statuses: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Index {
    pub(super) vector: BTreeMap<String, Value>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) unique_id: String,
    pub(super) collection_name: String,
    pub(super) endpoint: String,
    #[serde(default)]
    pub(super) failures: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Role {
    pub(super) vector: BTreeMap<String, Value>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) unique_id: String,
    pub(super) account_id: String,
    pub(super) collection_arn: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Flow {
    pub(super) vector: BTreeMap<String, Value>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) unique_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Store {
    pub(super) vector: BTreeMap<String, Value>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) unique_id: String,
    pub(super) content: Option<String>,
    pub(super) filename: Option<String>,
    pub(super) content_type: Option<String>,
    #[serde(default)]
    pub(super) job_statuses: Vec<String>,
}

pub(super) fn failed(message: String) -> HelperResult {
    let error = HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    };
    Err(Box::new(error))
}

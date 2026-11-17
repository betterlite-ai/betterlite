use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) argument: Option<Json>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

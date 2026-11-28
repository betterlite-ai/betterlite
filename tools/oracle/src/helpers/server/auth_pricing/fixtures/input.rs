use std::collections::BTreeMap;

use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

use super::router::RouterFixture;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    pub(crate) function: Function,
    pub(crate) router: RouterFixture,
    #[serde(default)]
    pub(crate) model: Option<ModelField>,
    #[serde(default)]
    pub(crate) route: Option<String>,
    #[serde(default)]
    pub(crate) method: Option<String>,
    #[serde(default)]
    pub(crate) body: JsonObject,
    #[serde(default)]
    pub(crate) enforce: bool,
    #[serde(default)]
    pub(crate) reject: bool,
    #[serde(default)]
    pub(crate) max_budget: Option<f64>,
    #[serde(default)]
    pub(crate) global_spend: Option<f64>,
    #[serde(default)]
    pub(crate) project: Option<CachedProject>,
    #[serde(default)]
    pub(crate) token: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub(crate) spends: Vec<JsonData>,
}

impl Call {
    pub(crate) fn requested(&self) -> RequestedModels {
        match &self.model {
            Some(ModelField::One(name)) => RequestedModels::One(name.clone()),
            Some(ModelField::Many(names)) => RequestedModels::Many(names.clone()),
            None => RequestedModels::None,
        }
    }
}

// the request model as LiteLLM types it: one name, a list, or nothing at all
#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum ModelField {
    One(String),
    Many(Vec<String>),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    ZeroCost,
    Unpriced,
    RequestSkip,
    RequestChecks,
    Project,
}

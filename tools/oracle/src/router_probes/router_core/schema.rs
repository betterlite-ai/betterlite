use betterlite_models::model_registration::JsonObject;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub model_name: String,
    pub model: String,
    pub id: String,
    pub weight: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub model: String,
    #[serde(default)]
    pub excluded_deployment_ids: Vec<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Carrier {
    Dict,
    Attributed,
    Stream,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub carrier: Carrier,
    pub hidden: Option<JsonObject>,
    pub metadata: Option<JsonObject>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selected {
    pub model_name: String,
    pub model: String,
    pub id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observed {
    pub error_type: String,
    pub model: String,
    pub cooldown_time: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Selected,
    BadRequest,
    RateLimited,
    ServiceUnavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    pub outcome: Outcome,
    pub selected: Option<Selected>,
    pub error: Option<Observed>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseFound {
    pub prepared: bool,
    pub has_hidden: bool,
    pub hidden: JsonObject,
}

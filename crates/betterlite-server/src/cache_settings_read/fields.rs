use serde::Serialize;
use serde_json::Value;

pub mod cloud;
pub mod common;
pub mod modes;
mod tests;

// LiteLLM's CacheSettingsField: the admin UI's description of one cache setting, plus the value a read filled in.
#[derive(Clone, Debug, Serialize)]
pub struct CacheSettingsField {
    pub field_name: &'static str,
    pub field_type: &'static str,
    pub field_value: Option<Value>,
    pub field_description: &'static str,
    pub field_default: FieldDefault,
    pub options: Option<&'static [&'static str]>,
    pub ui_field_name: &'static str,
    pub link: Option<&'static str>,
    pub redis_type: Option<&'static str>,
}

// LiteLLM types the default as Any: null, a bool, a string or a float, and only these four appear in the catalog.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(untagged)]
pub enum FieldDefault {
    Null,
    Bool(bool),
    Text(&'static str),
    Float(f64),
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct RedisTypeDescriptions {
    pub node: &'static str,
    pub cluster: &'static str,
    pub sentinel: &'static str,
}

pub const REDIS_TYPE_DESCRIPTIONS: RedisTypeDescriptions = RedisTypeDescriptions {
    node: "Standard Redis node/single instance",
    cluster: "Redis Cluster mode for high availability and horizontal scaling",
    sentinel: "Redis Sentinel mode for high availability with automatic failover",
};

// A fresh catalog per call, so one request's field_value never reaches the next request.
#[must_use]
pub fn catalog() -> Vec<CacheSettingsField> {
    let mut fields = Vec::new();
    fields.extend_from_slice(&common::CATALOG);
    fields.extend_from_slice(&modes::CATALOG);
    fields.extend_from_slice(&cloud::CATALOG);
    fields
}

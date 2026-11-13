use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

// every by-id interaction request carries the empty JSON object, so this body has no fields to set
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct EmptyBody(IndexMap<String, Json>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LifecycleRequest {
    pub url: String,
    pub body: EmptyBody,
}

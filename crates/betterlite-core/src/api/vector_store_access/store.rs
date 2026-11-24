use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};

use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use super::decode::parse_params;

// LiteLLM_ManagedVectorStore as stored: the fields this policy reads and every other one kept as given
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ManagedVectorStore(pub BTreeMap<String, Json>);

impl Debug for ManagedVectorStore {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // litellm_params carries a provider credential, so Debug names the fields and prints no value
        let names: Vec<&str> = self.0.keys().map(String::as_str).collect();
        f.debug_struct("ManagedVectorStore")
            .field("fields", &names)
            .field("litellm_params", &self.params_kind())
            .finish()
    }
}

impl ManagedVectorStore {
    // `vector_store.get("vector_store_id") or ""`: only text names a store, anything else compares as ""
    #[must_use]
    pub fn grant_id(&self) -> &str {
        if let Some(Json::Text(text)) = self.0.get("vector_store_id") {
            return text.as_str();
        }
        ""
    }

    // `vector_store.get("team_id") is None`: only a missing or null team id is the teamless case
    #[must_use]
    pub fn team_id_is_null_or_absent(&self) -> bool {
        match self.0.get("team_id") {
            None | Some(Json::Null) => true,
            Some(
                Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => false,
        }
    }

    // the store's team id when stored as text, the only value a text auth team id can equal
    #[must_use]
    pub fn team_id_text(&self) -> Option<&str> {
        match self.0.get("team_id") {
            Some(Json::Text(text)) => Some(text.as_str()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        }
    }

    fn params_kind(&self) -> &'static str {
        match self.0.get("litellm_params") {
            None => "absent",
            Some(Json::Object(_)) => "object",
            Some(Json::Text(_)) => "text",
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) => "other",
        }
    }
}

// _normalize_litellm_params: stored text becomes the object it encodes, or {} when it encodes none
#[must_use]
pub fn normalize_litellm_params(vector_store: &ManagedVectorStore) -> ManagedVectorStore {
    let mut normalized = vector_store.clone();
    if let Some(Json::Text(text)) = vector_store.0.get("litellm_params") {
        normalized.0.insert(
            "litellm_params".to_owned(),
            Json::Object(parse_params(text.as_bytes())),
        );
    }
    normalized
}

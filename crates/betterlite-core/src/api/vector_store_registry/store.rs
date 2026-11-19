use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_config::api::vector_store_registry::RegistryEntry;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use chrono::{DateTime, SecondsFormat, Utc};

use super::diagnostics;
use super::error::VectorStoreRegistryError;
use super::repository::{VectorStoreDatabase, stores_from_database};

pub type RegistryClock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

// CredentialAccessor.get_credential_values, injected so a test reads a snapshot it fixed itself
pub trait VectorStoreCredentials: Send + Sync {
    fn credential_values(&self, credential_name: &str) -> JsonObject;
}

impl VectorStoreCredentials for CredentialSnapshot {
    fn credential_values(&self, credential_name: &str) -> JsonObject {
        self.values(credential_name)
    }
}

pub struct VectorStoreRegistry {
    pub stores: Vec<LiteLlmManagedVectorStore>,
    clock: RegistryClock,
    credentials: Arc<dyn VectorStoreCredentials>,
}

impl Default for VectorStoreRegistry {
    fn default() -> Self {
        Self {
            stores: Vec::new(),
            clock: Arc::new(Utc::now),
            credentials: Arc::new(CredentialSnapshot::default()),
        }
    }
}

impl VectorStoreRegistry {
    #[must_use]
    pub const fn new(
        stores: Vec<LiteLlmManagedVectorStore>,
        clock: RegistryClock,
        credentials: Arc<dyn VectorStoreCredentials>,
    ) -> Self {
        Self {
            stores,
            clock,
            credentials,
        }
    }

    // load_vector_stores_from_config: each entry appends as it converts, so a bad one leaves the earlier records and raises without logging
    pub fn load_vector_stores_from_config(
        &mut self,
        entries: &[RegistryEntry],
    ) -> Result<(), VectorStoreRegistryError> {
        for entry in entries {
            let record = config_record(entry, &self.clock)?;
            self.stores.push(record);
        }
        tracing::debug!(
            "all loaded vector stores = {}",
            diagnostics::redacted_stores(&self.stores)
        );
        Ok(())
    }

    // get_litellm_managed_vector_store_from_registry: the first record carrying the id answers
    #[must_use]
    pub fn store_by_id(&self, vector_store_id: &str) -> Option<&LiteLlmManagedVectorStore> {
        self.stores
            .iter()
            .find(|store| store.vector_store_id.as_deref() == Some(vector_store_id))
    }

    // get_litellm_managed_vector_store_from_registry_or_db: memory first, then a database hit that joins the registry, and a failure logs at debug and answers None
    pub async fn lookup_or_database<D>(
        &mut self,
        vector_store_id: &str,
        database: Option<&D>,
    ) -> Option<LiteLlmManagedVectorStore>
    where
        D: VectorStoreDatabase,
    {
        if let Some(store) = self.store_by_id(vector_store_id) {
            return Some(store.clone());
        }
        let database = database?;
        let stores = match stores_from_database(Some(database)).await {
            Ok(stores) => stores,
            Err(error) => {
                tracing::debug!("Error fetching vector store from database: {error}");
                return None;
            }
        };
        let found = stores
            .into_iter()
            .find(|store| store.vector_store_id.as_deref() == Some(vector_store_id));
        if let Some(store) = &found {
            self.add_store(store.clone());
        }
        found
    }

    #[must_use]
    pub fn is_config(&self, vector_store_id: &str) -> bool {
        self.store_by_id(vector_store_id)
            .is_some_and(|store| store.is_config == Some(true))
    }

    // add_vector_store_to_registry: an id already present, including the absent one, is not added
    pub fn add_store(&mut self, store: LiteLlmManagedVectorStore) {
        for existing in &self.stores {
            if existing.vector_store_id == store.vector_store_id {
                return;
            }
        }
        self.stores.push(store);
    }

    // delete_vector_store_from_registry: every record carrying the id goes, config-owned included
    pub fn remove_store(&mut self, vector_store_id: &str) {
        let stores = std::mem::take(&mut self.stores);
        for store in stores {
            if store.vector_store_id.as_deref() != Some(vector_store_id) {
                self.stores.push(store);
            }
        }
    }

    // update_vector_store_in_registry: the record is replaced in place, a config-owned one is left untouched, and a missing id appends the update under the id it carries
    pub fn update_store(&mut self, vector_store_id: &str, updated: LiteLlmManagedVectorStore) {
        let position = self
            .stores
            .iter()
            .position(|store| store.vector_store_id.as_deref() == Some(vector_store_id));
        let Some(index) = position else {
            self.stores.push(updated);
            return;
        };
        let Some(store) = self.stores.get_mut(index) else {
            return;
        };
        if store.is_config != Some(true) {
            *store = updated;
        }
    }

    // get_credentials_for_vector_store: the first nonempty litellm_credential_name reaches the accessor, and inline litellm_params.api_key is never returned
    #[must_use]
    pub fn credentials_for_vector_store(&self, vector_store_id: &str) -> JsonObject {
        self.stores
            .iter()
            .filter(|store| store.vector_store_id.as_deref() == Some(vector_store_id))
            .filter_map(|store| store.litellm_credential_name.as_deref())
            .find(|name| !name.is_empty())
            .map_or_else(JsonObject::new, |name| {
                self.credentials.credential_values(name)
            })
    }
}

impl fmt::Debug for VectorStoreRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "VectorStoreRegistry {{ stores: {} }}",
            diagnostics::redacted_stores(&self.stores)
        )
    }
}

struct ConfigFields {
    id: String,
    provider: String,
    description: Option<String>,
}

fn config_fields(params: &JsonObject) -> Result<ConfigFields, VectorStoreRegistryError> {
    let id = match params.get("vector_store_id") {
        None | Some(Json::Null) => return Err(VectorStoreRegistryError::MissingVectorStoreId),
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(VectorStoreRegistryError::ParamType {
                field: "vector_store_id",
            });
        }
    };
    let provider = match params.get("custom_llm_provider") {
        None | Some(Json::Null) => {
            return Err(VectorStoreRegistryError::MissingCustomLlmProvider);
        }
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(VectorStoreRegistryError::ParamType {
                field: "custom_llm_provider",
            });
        }
    };
    let description = match params.get("vector_store_description") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(VectorStoreRegistryError::ParamType {
                field: "vector_store_description",
            });
        }
    };
    Ok(ConfigFields {
        id,
        provider,
        description,
    })
}

fn config_record(
    entry: &RegistryEntry,
    clock: &RegistryClock,
) -> Result<LiteLlmManagedVectorStore, VectorStoreRegistryError> {
    let empty = JsonObject::new();
    let params = entry.litellm_params.as_ref().unwrap_or(&empty);
    let fields = config_fields(params)?;
    let metadata = match params.get("vector_store_metadata") {
        None | Some(Json::Null) => None,
        Some(value @ (Json::Object(_) | Json::Text(_))) => Some(serde_json::to_value(value)?),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            return Err(VectorStoreRegistryError::ParamType {
                field: "vector_store_metadata",
            });
        }
    };
    Ok(LiteLlmManagedVectorStore {
        vector_store_id: Some(fields.id),
        custom_llm_provider: Some(fields.provider),
        vector_store_name: entry.vector_store_name.clone(),
        vector_store_description: fields.description,
        vector_store_metadata: metadata,
        created_at: Some(clock().to_rfc3339_opts(SecondsFormat::Micros, true)),
        updated_at: Some(clock().to_rfc3339_opts(SecondsFormat::Micros, true)),
        litellm_credential_name: None,
        litellm_params: Some(serde_json::to_value(params)?),
        team_id: None,
        user_id: None,
        is_config: Some(true),
        extra: BTreeMap::new(),
    })
}

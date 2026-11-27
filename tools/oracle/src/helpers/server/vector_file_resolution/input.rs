use std::collections::BTreeMap;

use betterlite_core::api::request_body::JsonObject as RequestBody;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

// the router's answers per model name; an absent name means no deployment credentials
pub(super) type CatalogScript = IndexMap<String, JsonObject>;
// the fields UserAPIKeyAuth::construct reads, as B1020 and B1061 read them
pub(super) type CallerScript = BTreeMap<String, JsonData>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(super) enum Call {
    UnifiedFileRouting(Routing),
    RestoreFileIdentifiers(Restore),
    ModelHintPrecedence(Hint),
    TeamModelFallback(Hint),
    UnifiedStoreAndRegistryOverlay(Overlay),
    ProviderAndIndexPermissions(Permissions),
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RequestScript {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) query: Option<String>,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: RequestBody,
}

impl Default for RequestScript {
    fn default() -> Self {
        Self {
            method: String::from("GET"),
            path: String::from("/v1/vector_stores/vs_1/files"),
            query: None,
            headers: Vec::new(),
            body: RequestBody::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Routing {
    pub(super) data: JsonObject,
    pub(super) file_id: String,
    #[serde(default)]
    pub(super) request: RequestScript,
    pub(super) caller: CallerScript,
    #[serde(default)]
    pub(super) catalog: Option<CatalogScript>,
    #[serde(default)]
    pub(super) deny: Vec<String>,
    #[serde(default)]
    pub(super) managed_files: Option<bool>,
    #[serde(default)]
    pub(super) require_managed_files: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Hint {
    pub(super) data: JsonObject,
    #[serde(default)]
    pub(super) request: RequestScript,
    #[serde(default)]
    pub(super) caller: Option<CallerScript>,
    #[serde(default)]
    pub(super) catalog: Option<CatalogScript>,
    #[serde(default)]
    pub(super) deny: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Overlay {
    pub(super) data: JsonObject,
    pub(super) vector_store_id: String,
    #[serde(default)]
    pub(super) managed_store: Option<Value>,
    // None is litellm.vector_store_registry being unset
    #[serde(default)]
    pub(super) registry: Option<Vec<Value>>,
    #[serde(default)]
    pub(super) should_lookup_registry: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Permissions {
    pub(super) data: JsonObject,
    pub(super) vector_store_id: String,
    #[serde(default)]
    pub(super) request: RequestScript,
    pub(super) caller: CallerScript,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Restore {
    pub(super) original_file_id: String,
    pub(super) response: ResponseScript,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum ResponseScript {
    Null,
    Dict {
        fields: JsonObject,
    },
    Object {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        file_id: Option<String>,
    },
}

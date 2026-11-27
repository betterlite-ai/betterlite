use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::route_match::canonical::RequestRouteSource;
use betterlite_context::resource_config_selection::vector_stores::{
    VectorStoreFilesConfigTag, VectorStoresConfigTag,
};
use betterlite_core::api::file_credentials::catalog::{CredentialCatalog, DeploymentRow};
use betterlite_core::api::file_credentials::routing::{GrantRequest, ModelGrant};
use betterlite_core::api::file_routing_ids::ManagedResourceAccessChecker;
use betterlite_core::api::model_credentials::CredentialSource;
use betterlite_core::api::request_body::JsonObject as RequestBody;
use betterlite_core::api::vector_endpoint_access::VectorEndpointTables;
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::vector::{
    LiteLlmManagedVectorStore, VectorStoreIndexEndpoints,
};
use betterlite_openai::vector_stores::{file_endpoints, store_endpoints};
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;

use crate::api::vector_file_resolution::error::VectorFileResolutionError;
use crate::api::vector_file_resolution::{RequestFacts, RoutingDeps, StoreDeps};

pub(super) const FILE_PATH: &str = "/v1/vector_stores/vs_1/files";

pub(super) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

pub(super) fn encoded(text: &str) -> String {
    URL_SAFE_NO_PAD.encode(text)
}

pub(super) fn store(text: &str) -> LiteLlmManagedVectorStore {
    serde_json::from_str(text).unwrap()
}

fn payload(text: &str) -> Payload {
    serde_json::from_str(text).unwrap()
}

pub(super) fn texts(values: &[&str]) -> JsonData {
    JsonData::Array(
        values
            .iter()
            .map(|value| JsonData::String((*value).to_owned()))
            .collect(),
    )
}

pub(super) fn auth(fields: &[(&str, JsonData)]) -> UserAPIKeyAuth {
    let mut entries: BTreeMap<String, JsonData> = BTreeMap::new();
    for (name, value) in fields {
        entries.insert((*name).to_owned(), value.clone());
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(entries)).unwrap()
}

pub(super) fn team_auth(team_models: JsonData, team_id: Option<&str>) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    fields.insert(String::from("team_models"), team_models);
    if let Some(team) = team_id {
        fields.insert(String::from("team_id"), JsonData::String(team.to_owned()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(super) fn gated_auth(metadata: Option<&str>, team_metadata: Option<&str>) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    if let Some(text) = metadata {
        fields.insert(String::from("metadata"), JsonData::Object(payload(text).0));
    }
    if let Some(text) = team_metadata {
        fields.insert(
            String::from("team_metadata"),
            JsonData::Object(payload(text).0),
        );
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(super) fn principal() -> Principal {
    serde_json::from_str(r#"{"principal_type":"human","subject":"u","auth_method":"api_key"}"#)
        .unwrap()
}

pub(super) fn header_map(pairs: &[(&str, &str)]) -> HeaderMap {
    pairs
        .iter()
        .map(|(name, value)| {
            (
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_bytes(value.as_bytes()).unwrap(),
            )
        })
        .collect()
}

fn locked<Slot>(slot: &Mutex<Slot>) -> MutexGuard<'_, Slot> {
    match slot.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// The request pieces a route hands to the helpers besides the decoded data; a test builds one, then lends its facts.
pub(super) struct Probe {
    pub(super) method: &'static str,
    pub(super) query: Option<&'static str>,
    pub(super) headers: HeaderMap,
    pub(super) body: RequestBody,
}

impl Probe {
    pub(super) fn get() -> Self {
        Self {
            method: "GET",
            query: None,
            headers: HeaderMap::new(),
            body: RequestBody::new(),
        }
    }

    pub(super) fn facts<'probe>(&'probe self, path: &'probe str) -> RequestFacts<'probe> {
        RequestFacts {
            method: self.method,
            query: self.query,
            headers: &self.headers,
            body: &self.body,
            route: RequestRouteSource {
                scope: None,
                url_path: path,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LookupCall {
    pub(super) model_id: String,
    pub(super) team_id: Option<String>,
}

#[derive(Default)]
pub(super) struct Catalog {
    credentials: IndexMap<String, JsonObject>,
    lookups: Mutex<Vec<LookupCall>>,
}

impl Catalog {
    pub(super) fn with(pairs: &[(&str, &str)]) -> Self {
        let mut catalog = Catalog::default();
        for (model_id, text) in pairs {
            catalog
                .credentials
                .insert((*model_id).to_owned(), object(text));
        }
        catalog
    }

    pub(super) fn lookups(&self) -> Vec<LookupCall> {
        locked(&self.lookups).clone()
    }
}

impl CredentialSource for Catalog {
    type Error = Infallible;

    fn credentials_with_provider(
        &self,
        model_id: &str,
        team_id: Option<&str>,
    ) -> Result<Option<JsonObject>, Infallible> {
        locked(&self.lookups).push(LookupCall {
            model_id: model_id.to_owned(),
            team_id: team_id.map(str::to_owned),
        });
        Ok(self.credentials.get(model_id).cloned())
    }
}

impl CredentialCatalog for Catalog {
    fn model_names(&self, _team_id: Option<&str>) -> Result<Vec<String>, Infallible> {
        Ok(Vec::new())
    }

    fn model_access_groups(&self) -> Result<IndexMap<String, Vec<String>>, Infallible> {
        Ok(IndexMap::new())
    }

    fn deployments(&self) -> Result<Vec<DeploymentRow>, Infallible> {
        Ok(Vec::new())
    }

    fn resolved_model_info(&self, _model_id: &str) -> Result<Option<JsonObject>, Infallible> {
        Ok(None)
    }
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("the scripted grant denied `{0}`")]
pub(super) struct Denied(String);

#[derive(Default)]
pub(super) struct Grants {
    asked: Mutex<Vec<String>>,
    denial: Option<Denied>,
}

impl Grants {
    pub(super) fn denying(model: &str) -> Self {
        Self {
            asked: Mutex::new(Vec::new()),
            denial: Some(Denied(model.to_owned())),
        }
    }

    pub(super) fn asked(&self) -> Vec<String> {
        locked(&self.asked).clone()
    }
}

#[async_trait]
impl ModelGrant<Catalog> for Grants {
    type Error = Denied;

    async fn authorize(&self, request: &GrantRequest<'_, Catalog>) -> Result<(), Denied> {
        locked(&self.asked).push(request.model.to_owned());
        match &self.denial {
            Some(denied) => Err(denied.clone()),
            None => Ok(()),
        }
    }
}

#[derive(Default)]
pub(super) struct Checker {
    allow: bool,
    calls: Mutex<Vec<String>>,
}

impl Checker {
    pub(super) const fn allowing() -> Self {
        Self {
            allow: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        locked(&self.calls).clone()
    }
}

#[async_trait]
impl ManagedResourceAccessChecker for Checker {
    async fn can_call_unified_file_id(
        &self,
        unified_file_id: &str,
        _principal: &Principal,
    ) -> bool {
        locked(&self.calls).push(unified_file_id.to_owned());
        self.allow
    }

    async fn can_call_unified_object_id(&self, object_id: &str, principal: &Principal) -> bool {
        self.can_call_unified_file_id(object_id, principal).await
    }
}

pub(super) struct Tables;

impl VectorEndpointTables for Tables {
    fn store_endpoints(&self, _config: VectorStoresConfigTag) -> VectorStoreIndexEndpoints {
        store_endpoints()
    }

    fn file_endpoints(&self, _config: VectorStoreFilesConfigTag) -> VectorStoreIndexEndpoints {
        file_endpoints()
    }
}

pub(super) static TABLES: Tables = Tables;

pub(super) fn routing_deps<'deps>(
    catalog: Option<&'deps Catalog>,
    grants: &'deps Grants,
    managed_files: Option<&'deps dyn ManagedResourceAccessChecker>,
    require_managed_files: bool,
) -> RoutingDeps<'deps, Catalog, Grants> {
    RoutingDeps {
        catalog,
        grants,
        managed_files,
        require_managed_files,
    }
}

pub(super) fn store_deps<'deps>(
    managed_store: Option<&'deps LiteLlmManagedVectorStore>,
    registry: Option<&'deps VectorStoreRegistry>,
    should_lookup_registry: bool,
) -> StoreDeps<'deps> {
    StoreDeps {
        managed_store,
        registry,
        should_lookup_registry,
    }
}

// The route's error, or a panic naming what answered instead; a route that should fail never returns Ok.
pub(super) fn failure<Answer>(
    result: Result<Answer, VectorFileResolutionError<Infallible, Denied>>,
) -> VectorFileResolutionError<Infallible, Denied> {
    match result {
        Err(found) => found,
        Ok(_) => panic!("the route should have failed"),
    }
}

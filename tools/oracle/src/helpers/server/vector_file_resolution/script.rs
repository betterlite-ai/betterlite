use std::convert::Infallible;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
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
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreIndexEndpoints;
use betterlite_openai::vector_stores::{file_endpoints, store_endpoints};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_server::api::vector_file_resolution::RequestFacts;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use serde_json::{Value, json};
use thiserror::Error;

use super::input::{CallerScript, CatalogScript, RequestScript};

pub(super) fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "Internal".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

// the request the case scripts, turned into the facts a files route hands the helpers
pub(super) struct Prepared {
    method: String,
    path: String,
    query: Option<String>,
    headers: HeaderMap,
    body: RequestBody,
}

impl Prepared {
    pub(super) fn new(script: &RequestScript) -> Result<Self, Box<HelperError>> {
        let mut headers = HeaderMap::new();
        for (name, value) in &script.headers {
            headers.append(
                HeaderName::from_bytes(name.as_bytes()).or_value_error()?,
                HeaderValue::from_bytes(value.as_bytes()).or_value_error()?,
            );
        }
        Ok(Self {
            method: script.method.clone(),
            path: script.path.clone(),
            query: script.query.clone(),
            headers,
            body: script.body.clone(),
        })
    }

    pub(super) fn facts(&self) -> RequestFacts<'_> {
        RequestFacts {
            method: &self.method,
            query: self.query.as_deref(),
            headers: &self.headers,
            body: &self.body,
            route: RequestRouteSource {
                scope: None,
                url_path: &self.path,
            },
        }
    }
}

pub(super) fn key_auth(script: &CallerScript) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    UserAPIKeyAuth::construct(AuthInput::Dict(script.clone())).or_value_error()
}

pub(super) fn caller_principal() -> Result<Principal, Box<HelperError>> {
    Ok(serde_json::from_str(
        r#"{"principal_type":"human","subject":"u","auth_method":"api_key"}"#,
    )?)
}

#[derive(Clone)]
struct LookupCall {
    model_id: String,
    team_id: Option<String>,
}

// the router's deployment credentials per model name, recording every lookup in order
pub(super) struct Catalog {
    entries: CatalogScript,
    lookups: Mutex<Vec<LookupCall>>,
}

impl Catalog {
    pub(super) fn new(entries: CatalogScript) -> Self {
        Self {
            entries,
            lookups: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn lookups_json(&self) -> Value {
        let calls = locked(&self.lookups);
        Value::Array(
            calls
                .iter()
                .map(|call| json!({"model_id": call.model_id, "team_id": call.team_id}))
                .collect(),
        )
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
        Ok(self.entries.get(model_id).cloned())
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

#[derive(Clone, Debug, Error)]
#[error("the scripted grant denied `{0}`")]
pub(super) struct Denied(String);

// the model grant B1020 calls, denying the names the case lists
pub(super) struct Grants {
    denied: Vec<String>,
    asked: Mutex<Vec<String>>,
}

impl Grants {
    pub(super) fn new(denied: Vec<String>) -> Self {
        Self {
            denied,
            asked: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn asked_json(&self) -> Value {
        Value::Array(
            locked(&self.asked)
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        )
    }
}

#[async_trait]
impl ModelGrant<Catalog> for Grants {
    type Error = Denied;

    async fn authorize(&self, request: &GrantRequest<'_, Catalog>) -> Result<(), Denied> {
        locked(&self.asked).push(request.model.to_owned());
        if self
            .denied
            .iter()
            .any(|model| model.as_str() == request.model)
        {
            return Err(Denied(request.model.to_owned()));
        }
        Ok(())
    }
}

// B1021's ownership hook, answering with the case's single allow flag
pub(super) struct Checker {
    allow: bool,
}

impl Checker {
    pub(super) const fn new(allow: bool) -> Self {
        Self { allow }
    }
}

#[async_trait]
impl ManagedResourceAccessChecker for Checker {
    async fn can_call_unified_file_id(
        &self,
        _unified_file_id: &str,
        _principal: &Principal,
    ) -> bool {
        self.allow
    }

    async fn can_call_unified_object_id(
        &self,
        _unified_object_id: &str,
        _principal: &Principal,
    ) -> bool {
        self.allow
    }
}

// B1061's endpoint tables as the proxy configures them
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

fn locked<Slot>(slot: &Mutex<Slot>) -> MutexGuard<'_, Slot> {
    match slot.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

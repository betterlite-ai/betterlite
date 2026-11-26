use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::{Principal, SourceKey};
use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_auth::auth_model_lists::parameters::ModelParameters;
use betterlite_auth::auth_model_lists::providers::ProviderModels;
use betterlite_auth::auth_model_lists::wildcard::ExpansionContext;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::api::file_credentials::catalog::{CredentialCatalog, DeploymentRow};
use betterlite_core::api::file_credentials::routing::{GrantRequest, ModelGrant};
use betterlite_core::api::model_credentials::CredentialSource;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;

use super::failure;
use super::input::{KeyInput, Lookup, RouterInput};

pub(crate) struct ScriptedCatalog<'script> {
    script: &'script RouterInput,
    pub(crate) lookups: Mutex<Vec<Lookup>>,
}

impl<'script> ScriptedCatalog<'script> {
    pub(crate) const fn new(script: &'script RouterInput) -> Self {
        Self {
            script,
            lookups: Mutex::new(Vec::new()),
        }
    }
}

impl CredentialSource for ScriptedCatalog<'_> {
    // the scripted router answers what the case says and cannot fail
    type Error = Infallible;

    fn credentials_with_provider(
        &self,
        model_id: &str,
        team_id: Option<&str>,
    ) -> Result<Option<JsonObject>, Infallible> {
        locked(&self.lookups).push(Lookup {
            model_id: model_id.to_owned(),
            team_id: team_id.map(str::to_owned),
        });
        Ok(self.script.credentials.get(model_id).cloned())
    }
}

impl CredentialCatalog for ScriptedCatalog<'_> {
    fn model_names(&self, _team_id: Option<&str>) -> Result<Vec<String>, Infallible> {
        Ok(self.script.model_names.clone())
    }

    fn model_access_groups(&self) -> Result<IndexMap<String, Vec<String>>, Infallible> {
        Ok(self.script.access_groups.clone())
    }

    fn deployments(&self) -> Result<Vec<DeploymentRow>, Infallible> {
        let rows = self
            .script
            .deployments
            .iter()
            .map(|row| DeploymentRow {
                model_name: row.model_name.clone(),
                model_info: row.model_info.clone(),
            })
            .collect();
        Ok(rows)
    }

    fn resolved_model_info(&self, model_id: &str) -> Result<Option<JsonObject>, Infallible> {
        Ok(self.script.credential_deployments.get(model_id).cloned())
    }
}

#[derive(Default)]
struct NoProviders;

impl ProviderModels for NoProviders {
    fn valid_models(
        &self,
        _provider: Option<&str>,
        _parameters: Option<&ModelParameters>,
    ) -> Option<Vec<String>> {
        None
    }
}

// the wildcard expansion these cases use: no provider catalogue and no named credentials
#[derive(Default)]
pub(crate) struct ScriptedExpansion {
    providers: NoProviders,
    credentials: CredentialSnapshot,
}

impl ScriptedExpansion {
    pub(crate) fn context(&self) -> ExpansionContext<'_> {
        ExpansionContext {
            providers: &self.providers,
            credentials: &self.credentials,
            deployments: None,
        }
    }
}

// every oracle case authorizes: the denial path is the native tests' job
pub(crate) struct AllowAll;

#[async_trait]
impl<'script> ModelGrant<ScriptedCatalog<'script>> for AllowAll {
    type Error = Infallible;

    async fn authorize(
        &self,
        _request: &GrantRequest<'_, ScriptedCatalog<'script>>,
    ) -> Result<(), Infallible> {
        Ok(())
    }
}

pub(crate) struct Scenario<'script> {
    pub(crate) catalog: Option<ScriptedCatalog<'script>>,
    pub(crate) grants: AllowAll,
    pub(crate) identity: Principal,
    pub(crate) expansion: ScriptedExpansion,
}

pub(crate) fn scenario<'script>(
    key: &KeyInput,
    router: Option<&'script RouterInput>,
) -> Result<Scenario<'script>, Box<HelperError>> {
    Ok(Scenario {
        catalog: router.map(ScriptedCatalog::new),
        grants: AllowAll,
        identity: principal_for(key)?,
        expansion: ScriptedExpansion::default(),
    })
}

pub(super) fn locked<Slot>(slot: &Mutex<Slot>) -> MutexGuard<'_, Slot> {
    match slot.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn key_texts(values: &[String]) -> JsonData {
    let items = values
        .iter()
        .map(|value| JsonData::String(value.clone()))
        .collect();
    JsonData::Array(items)
}

fn principal_for(key: &KeyInput) -> Result<Principal, Box<HelperError>> {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    fields.insert("models".to_owned(), key_texts(&key.models));
    fields.insert("team_models".to_owned(), key_texts(&key.team_models));
    if let Some(team) = &key.team_id {
        fields.insert("team_id".to_owned(), JsonData::String(team.clone()));
    }
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields))
        .map_err(|error| failure(error.to_string()))?;
    let mut identity: Principal = serde_json::from_str(
        r#"{"principal_type":"human","subject":"scripted","auth_method":"api_key"}"#,
    )?;
    identity.source_key = SourceKey(Some(auth));
    Ok(identity)
}

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::{Principal, SourceKey};
use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_auth::auth_model_lists::parameters::ModelParameters;
use betterlite_auth::auth_model_lists::providers::ProviderModels;
use betterlite_auth::auth_model_lists::wildcard::ExpansionContext;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use crate::api::file_credentials::catalog::{CredentialCatalog, DeploymentRow};
use crate::api::file_credentials::hints::ModelHints;
use crate::api::file_credentials::routing::{GrantRequest, ModelGrant};
use crate::api::model_credentials::CredentialSource;

pub(super) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

pub(super) fn credentials(pairs: &[(&str, &str)]) -> IndexMap<String, JsonObject> {
    pairs
        .iter()
        .map(|(model_id, text)| ((*model_id).to_owned(), object(text)))
        .collect()
}

pub(super) fn row(model_name: &str, model_info: &str) -> DeploymentRow {
    DeploymentRow {
        model_name: model_name.to_owned(),
        model_info: object(model_info),
    }
}

pub(super) fn locked<Value>(slot: &Mutex<Value>) -> MutexGuard<'_, Value> {
    match slot.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// The order of the grant and the credential lookup, shared by both scripted ports.
#[derive(Clone, Default)]
pub(super) struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    pub(super) fn push(&self, event: String) {
        locked(&self.0).push(event);
    }

    pub(super) fn entries(&self) -> Vec<String> {
        locked(&self.0).clone()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("scripted catalog failure")]
pub(super) struct Fail;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LookupCall {
    pub(super) model_id: String,
    pub(super) team_id: Option<String>,
}

#[derive(Default)]
pub(super) struct Catalog {
    pub(super) model_names: Vec<String>,
    pub(super) access_groups: IndexMap<String, Vec<String>>,
    pub(super) deployments: Vec<DeploymentRow>,
    pub(super) resolved: IndexMap<String, JsonObject>,
    pub(super) credentials: IndexMap<String, JsonObject>,
    pub(super) events: Log,
    pub(super) lookups: Mutex<Vec<LookupCall>>,
    pub(super) name_calls: Mutex<Vec<Option<String>>>,
    pub(super) fail_lookup: bool,
}

impl CredentialSource for Catalog {
    type Error = Fail;

    fn credentials_with_provider(
        &self,
        model_id: &str,
        team_id: Option<&str>,
    ) -> Result<Option<JsonObject>, Fail> {
        self.events.push(format!("lookup {model_id}"));
        locked(&self.lookups).push(LookupCall {
            model_id: model_id.to_owned(),
            team_id: team_id.map(str::to_owned),
        });
        if self.fail_lookup {
            return Err(Fail);
        }
        Ok(self.credentials.get(model_id).cloned())
    }
}

impl CredentialCatalog for Catalog {
    fn model_names(&self, team_id: Option<&str>) -> Result<Vec<String>, Fail> {
        locked(&self.name_calls).push(team_id.map(str::to_owned));
        Ok(self.model_names.clone())
    }

    fn model_access_groups(&self) -> Result<IndexMap<String, Vec<String>>, Fail> {
        Ok(self.access_groups.clone())
    }

    fn deployments(&self) -> Result<Vec<DeploymentRow>, Fail> {
        Ok(self.deployments.clone())
    }

    fn resolved_model_info(&self, model_id: &str) -> Result<Option<JsonObject>, Fail> {
        Ok(self.resolved.get(model_id).cloned())
    }
}

pub(super) fn lookup_calls(catalog: &Catalog) -> Vec<LookupCall> {
    locked(&catalog.lookups).clone()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GrantAsked {
    pub(super) model: String,
    pub(super) llm_model_list: Option<Vec<String>>,
    pub(super) subject: String,
    pub(super) router_present: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub(super) struct Denied {
    pub(super) status: u16,
    pub(super) error_type: String,
    pub(super) param: Option<String>,
    pub(super) code: Option<String>,
    pub(super) detail: String,
}

impl Denied {
    pub(super) fn forbidden() -> Self {
        Self {
            status: 403,
            error_type: "team_model_access_denied".to_owned(),
            param: Some("m".to_owned()),
            code: Some("key_model_access_denied".to_owned()),
            detail: "not allowed to call m".to_owned(),
        }
    }
}

#[derive(Default)]
pub(super) struct Grants {
    pub(super) events: Log,
    pub(super) denial: Option<Denied>,
    pub(super) asked: Mutex<Vec<GrantAsked>>,
}

#[async_trait]
impl ModelGrant<Catalog> for Grants {
    type Error = Denied;

    async fn authorize(&self, request: &GrantRequest<'_, Catalog>) -> Result<(), Denied> {
        self.events.push(format!("grant {}", request.model));
        locked(&self.asked).push(GrantAsked {
            model: request.model.to_owned(),
            llm_model_list: request.llm_model_list.map(|list| list.to_vec()),
            subject: request.principal.subject.clone(),
            router_present: request.router.is_some(),
        });
        match &self.denial {
            Some(denial) => Err(denial.clone()),
            None => Ok(()),
        }
    }
}

pub(super) fn granted(events: &Log) -> Grants {
    Grants {
        events: events.clone(),
        ..Grants::default()
    }
}

fn names(values: &[&str]) -> JsonData {
    JsonData::Array(
        values
            .iter()
            .map(|name| JsonData::String((*name).to_owned()))
            .collect(),
    )
}

fn key(models: &[&str], team_models: &[&str], team_id: Option<&str>) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    fields.insert("api_key".to_owned(), JsonData::String("sk-key".to_owned()));
    fields.insert("models".to_owned(), names(models));
    fields.insert("team_models".to_owned(), names(team_models));
    if let Some(team) = team_id {
        fields.insert("team_id".to_owned(), JsonData::String(team.to_owned()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(super) fn principal(models: &[&str], team_models: &[&str], team_id: Option<&str>) -> Principal {
    let mut identity: Principal = serde_json::from_str(
        r#"{"principal_type":"human","subject":"u","auth_method":"api_key","user":{"id":"u"}}"#,
    )
    .unwrap();
    identity.source_key = SourceKey(Some(key(models, team_models, team_id)));
    identity
}

pub(super) fn hints(body: Option<&str>, query: Option<&str>, header: Option<&str>) -> ModelHints {
    ModelHints {
        body: body.map(str::to_owned),
        query: query.map(str::to_owned),
        header: header.map(str::to_owned),
    }
}

pub(super) struct NoProviders;

impl ProviderModels for NoProviders {
    fn valid_models(
        &self,
        _provider: Option<&str>,
        _parameters: Option<&ModelParameters>,
    ) -> Option<Vec<String>> {
        None
    }
}

pub(super) struct ScriptedProviders {
    pub(super) provider: String,
    pub(super) models: Vec<String>,
    pub(super) asked: Mutex<Vec<Option<String>>>,
}

impl ProviderModels for ScriptedProviders {
    fn valid_models(
        &self,
        provider: Option<&str>,
        _parameters: Option<&ModelParameters>,
    ) -> Option<Vec<String>> {
        locked(&self.asked).push(provider.map(str::to_owned));
        (provider == Some(self.provider.as_str())).then(|| self.models.clone())
    }
}

pub(super) fn expansion<'context>(
    providers: &'context dyn ProviderModels,
    credentials: &'context CredentialSnapshot,
) -> ExpansionContext<'context> {
    ExpansionContext {
        providers,
        credentials,
        deployments: None,
    }
}

pub(super) fn team_catalog(events: &Log) -> Catalog {
    Catalog {
        model_names: Vec::new(),
        deployments: vec![
            row(
                "a",
                r#"{"id":"d1","team_id":"t","team_public_model_name":"a"}"#,
            ),
            row(
                "b",
                r#"{"id":"d2","team_id":"t","team_public_model_name":"b"}"#,
            ),
            row(
                "glob",
                r#"{"id":"d3","team_id":"other","team_public_model_name":"glob"}"#,
            ),
            row("no-id", r#"{"team_id":"t"}"#),
            row(
                "anthropic-row",
                r#"{"id":"d4","team_id":"t","team_public_model_name":"claude-x"}"#,
            ),
        ],
        credentials: credentials(&[
            (
                "d1",
                r#"{"api_key":"k1","api_base":"https://one","custom_llm_provider":"openai","model":"m1"}"#,
            ),
            (
                "d2",
                r#"{"api_key":"k2","api_base":"https://two","custom_llm_provider":"openai","model":"m2"}"#,
            ),
            ("d3", r#"{"api_key":"k3","custom_llm_provider":"openai"}"#),
            (
                "d4",
                r#"{"api_key":"k4","custom_llm_provider":"anthropic"}"#,
            ),
        ]),
        events: events.clone(),
        ..Catalog::default()
    }
}

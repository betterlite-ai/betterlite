use std::sync::Arc;

use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_config::sections::request_model_overrides::Settings as ModelOverridesSettings;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use crate::api::request_destination::RequestData;
use crate::api::request_model_overrides::catalog::{DeploymentCatalogView, DeploymentParams};
use crate::api::request_model_overrides::warnings::WarnOnce;
use crate::api::request_model_overrides::{
    MODEL_FIELD, OverridesPrincipal, RequestModelOverridesInput, RequestModelOverridesService,
};

// the three router indices the stale-alias and provider branches read, fixed by the test
#[derive(Debug, Default)]
pub(super) struct TestCatalog {
    groups: IndexMap<String, DeploymentParams>,
    team_groups: Vec<(String, String)>,
}

impl TestCatalog {
    pub(super) fn with_group(mut self, model_group: &str, params: DeploymentParams) -> Self {
        self.groups.insert(model_group.to_owned(), params);
        self
    }

    pub(super) fn with_team_group(mut self, team_id: &str, model_group: &str) -> Self {
        self.team_groups
            .push((team_id.to_owned(), model_group.to_owned()));
        self
    }
}

impl DeploymentCatalogView for TestCatalog {
    fn has_model_group(&self, model_group: &str) -> bool {
        self.groups.contains_key(model_group)
    }

    fn has_team_model_group(&self, team_id: &str, model_group: &str) -> bool {
        self.team_groups
            .iter()
            .any(|(team, group)| team == team_id && group == model_group)
    }

    fn deployment_params(&self, model_group: &str) -> Option<DeploymentParams> {
        self.groups.get(model_group).cloned()
    }
}

pub(super) fn deployment(provider: &str, model: &str) -> DeploymentParams {
    DeploymentParams {
        custom_llm_provider: Some(provider.to_owned()),
        model: Some(model.to_owned()),
    }
}

pub(super) fn catalog(catalog: TestCatalog) -> Option<Arc<dyn DeploymentCatalogView>> {
    Some(Arc::new(catalog))
}

pub(super) fn credential_settings() -> ModelOverridesSettings {
    ModelOverridesSettings {
        credential_overrides: true,
        enforced_params: None,
        service_account_params: None,
        team_stale_alias_bypass: false,
    }
}

pub(super) fn bypass_settings() -> ModelOverridesSettings {
    ModelOverridesSettings {
        credential_overrides: false,
        team_stale_alias_bypass: true,
        ..credential_settings()
    }
}

pub(super) fn enforced_settings(
    enforced: Option<&[&str]>,
    service_account: Option<&[&str]>,
) -> ModelOverridesSettings {
    ModelOverridesSettings {
        credential_overrides: false,
        enforced_params: texts(enforced),
        service_account_params: texts(service_account),
        team_stale_alias_bypass: false,
    }
}

pub(super) fn texts(values: Option<&[&str]>) -> Option<Vec<String>> {
    values.map(|items| items.iter().map(|item| (*item).to_owned()).collect())
}

pub(super) fn alias_map(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(from, to)| ((*from).to_owned(), (*to).to_owned()))
        .collect()
}

pub(super) fn credential(name: &str, values: &[(&str, &str)]) -> CredentialEntry {
    let mut credential_values = JsonObject::new();
    for (field, value) in values {
        credential_values.insert((*field).to_owned(), Json::Text((*value).to_owned()));
    }
    CredentialEntry {
        credential_name: name.to_owned(),
        credential_values,
    }
}

pub(super) fn build_service(
    settings: ModelOverridesSettings,
    catalog_view: Option<Arc<dyn DeploymentCatalogView>>,
    credentials: Vec<CredentialEntry>,
) -> RequestModelOverridesService {
    RequestModelOverridesService::new(
        settings,
        catalog_view,
        Arc::new(CredentialSnapshot::new(credentials)),
    )
}

pub(super) fn request(fields: &[(&str, &str)]) -> RequestData {
    fields
        .iter()
        .map(|(field, value)| ((*field).to_owned(), Json::Text((*value).to_owned())))
        .collect()
}

pub(super) fn requested(model: &str) -> RequestData {
    request(&[(MODEL_FIELD, model)])
}

// model_config: model -> provider -> litellm_credentials, where no name leaves that provider entry empty
pub(super) fn model_config(entries: &[(&str, &str, Option<&str>)]) -> JsonObject {
    let mut providers_by_model: IndexMap<String, JsonObject> = IndexMap::new();
    for (model, provider, name) in entries {
        let mut fields = JsonObject::new();
        if let Some(name) = name {
            fields.insert(
                "litellm_credentials".to_owned(),
                Json::Text((*name).to_owned()),
            );
        }
        providers_by_model
            .entry((*model).to_owned())
            .or_default()
            .insert((*provider).to_owned(), Json::Object(fields));
    }
    let config: JsonObject = providers_by_model
        .into_iter()
        .map(|(model, providers)| (model, Json::Object(providers)))
        .collect();
    JsonObject::from([("model_config".to_owned(), Json::Object(config))])
}

// one model_config provider entry whose field carries a value of the caller's type
pub(super) fn model_config_field(
    model: &str,
    provider: &str,
    field: &str,
    value: Json,
) -> JsonObject {
    let fields = JsonObject::from([(field.to_owned(), value)]);
    let providers = JsonObject::from([(provider.to_owned(), Json::Object(fields))]);
    let models = JsonObject::from([(model.to_owned(), Json::Object(providers))]);
    JsonObject::from([("model_config".to_owned(), Json::Object(models))])
}

pub(super) fn input(principal: OverridesPrincipal, model: &str) -> RequestModelOverridesInput {
    RequestModelOverridesInput {
        data: requested(model),
        principal,
        premium_user: true,
    }
}

pub(super) fn team_principal(team_aliases: &[(&str, &str)]) -> OverridesPrincipal {
    OverridesPrincipal {
        team_id: Some("team-1".to_owned()),
        team_model_aliases: alias_map(team_aliases),
        aliases: alias_map(&[]),
        metadata: None,
        team_metadata: None,
        project_metadata: None,
    }
}

pub(super) fn fill(warn_once: &WarnOnce, count: usize) {
    for index in 0..count {
        warn_once.first_seen(format!("key-{index}"));
    }
}

pub(super) fn text_field<'data>(data: &'data RequestData, field: &str) -> Option<&'data str> {
    match data.get(field) {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Bool(_) | Json::Null | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

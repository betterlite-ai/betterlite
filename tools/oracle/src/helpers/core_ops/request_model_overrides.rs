use std::sync::Arc;

use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_config::sections::request_model_overrides::Settings as ModelOverridesSettings;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_model_overrides::catalog::{
    DeploymentCatalogView, DeploymentParams,
};
use betterlite_core::api::request_model_overrides::diagnostics::OverrideDiagnostic;
use betterlite_core::api::request_model_overrides::error::ErrorFacts;
use betterlite_core::api::request_model_overrides::{
    OverridesPrincipal, RequestModelOverridesInput, RequestModelOverridesService,
};
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    settings: SettingsInput,
    #[serde(default)]
    deployments: Option<Vec<DeploymentInput>>,
    #[serde(default)]
    credentials: Vec<CredentialInput>,
    calls: Vec<CallInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsInput {
    credential_overrides: bool,
    #[serde(default)]
    enforced_params: Option<Vec<String>>,
    #[serde(default)]
    service_account_params: Option<Vec<String>>,
    #[serde(default)]
    team_stale_alias_bypass: bool,
}

// one Router model_list entry: model_name is the public group, model_info carries the team scope
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentInput {
    model_name: String,
    model: String,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    team_public_model_name: Option<String>,
}

// one litellm.credential_list entry the CredentialAccessor reads by name
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialInput {
    name: String,
    values: JsonObject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallInput {
    data: RequestData,
    principal: PrincipalInput,
    premium_user: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrincipalInput {
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    team_model_aliases: IndexMap<String, String>,
    #[serde(default)]
    aliases: IndexMap<String, String>,
    #[serde(default)]
    metadata: Option<JsonObject>,
    #[serde(default)]
    team_metadata: Option<JsonObject>,
    #[serde(default)]
    project_metadata: Option<JsonObject>,
}

// the Router indices the production trait reads, scripted from the case's model_list
struct ScriptedCatalog {
    deployments: Vec<DeploymentInput>,
}

impl DeploymentCatalogView for ScriptedCatalog {
    fn has_model_group(&self, model_group: &str) -> bool {
        self.deployments
            .iter()
            .any(|deployment| deployment.model_name == model_group)
    }

    fn has_team_model_group(&self, team_id: &str, model_group: &str) -> bool {
        self.deployments.iter().any(|deployment| {
            deployment.team_id.as_deref() == Some(team_id)
                && deployment.team_public_model_name.as_deref() == Some(model_group)
        })
    }

    fn deployment_params(&self, model_group: &str) -> Option<DeploymentParams> {
        self.deployments
            .iter()
            .find(|deployment| deployment.model_name == model_group)
            .map(|deployment| DeploymentParams {
                custom_llm_provider: deployment.custom_llm_provider.clone(),
                model: Some(deployment.model.clone()),
            })
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let service = build_service(input);
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(call_value(&service, call));
        }
        Ok(Value::Array(results))
    }
}

fn build_service(input: &Input) -> RequestModelOverridesService {
    let catalog = input.deployments.as_deref().map(scripted_catalog);
    let mut credentials = Vec::with_capacity(input.credentials.len());
    for credential in &input.credentials {
        credentials.push(CredentialEntry {
            credential_name: credential.name.clone(),
            credential_values: credential.values.clone(),
        });
    }
    RequestModelOverridesService::new(
        ModelOverridesSettings {
            credential_overrides: input.settings.credential_overrides,
            enforced_params: input.settings.enforced_params.clone(),
            service_account_params: input.settings.service_account_params.clone(),
            team_stale_alias_bypass: input.settings.team_stale_alias_bypass,
        },
        catalog,
        Arc::new(CredentialSnapshot::new(credentials)),
    )
}

fn scripted_catalog(deployments: &[DeploymentInput]) -> Arc<dyn DeploymentCatalogView> {
    Arc::new(ScriptedCatalog {
        deployments: deployments.to_vec(),
    })
}

fn call_value(service: &RequestModelOverridesService, call: &CallInput) -> Value {
    let input = RequestModelOverridesInput {
        data: call.data.clone(),
        principal: OverridesPrincipal {
            team_id: call.principal.team_id.clone(),
            team_model_aliases: call.principal.team_model_aliases.clone(),
            aliases: call.principal.aliases.clone(),
            metadata: call.principal.metadata.clone(),
            team_metadata: call.principal.team_metadata.clone(),
            project_metadata: call.principal.project_metadata.clone(),
        },
        premium_user: call.premium_user,
    };
    match service.apply(&input) {
        Ok(result) => json!({
            "data": result.data,
            "enforced_params": result.enforced_params,
            "diagnostics": diagnostics_json(&result.diagnostics),
        }),
        Err(error) => json!({ "error": facts_json(error.facts()) }),
    }
}

fn diagnostics_json(diagnostics: &[OverrideDiagnostic]) -> Vec<Value> {
    let mut values = Vec::with_capacity(diagnostics.len());
    for diagnostic in diagnostics {
        values.push(diagnostic_json(diagnostic));
    }
    values
}

fn diagnostic_json(diagnostic: &OverrideDiagnostic) -> Value {
    let level = diagnostic.level().as_str();
    match diagnostic {
        OverrideDiagnostic::StaleAliasMissingDeployment {
            model,
            team_id,
            aliased_target,
        } => json!({
            "kind": diagnostic.kind(),
            "level": level,
            "model": model,
            "team_id": team_id,
            "aliased_target": aliased_target,
        }),
        OverrideDiagnostic::StaleAliasSiblingDeployment { model, team_id } => json!({
            "kind": diagnostic.kind(),
            "level": level,
            "model": model,
            "team_id": team_id,
        }),
        OverrideDiagnostic::ModelEntryWithoutCredentials { model } => json!({
            "kind": diagnostic.kind(),
            "level": level,
            "model": model,
        }),
        OverrideDiagnostic::CredentialNotFound { credential } => json!({
            "kind": diagnostic.kind(),
            "level": level,
            "credential": credential,
        }),
        OverrideDiagnostic::CredentialApplied { credential, model } => json!({
            "kind": diagnostic.kind(),
            "level": level,
            "credential": credential,
            "model": model,
        }),
    }
}

fn facts_json(facts: ErrorFacts) -> Value {
    json!({
        "status": facts.status,
        "type": facts.error_type,
        "param": facts.param,
        "code": facts.code,
    })
}

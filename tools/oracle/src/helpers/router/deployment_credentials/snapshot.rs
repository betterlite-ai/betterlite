use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_credentials::resolve::{
    credentials_with_provider, deployment_credentials,
};
use betterlite_router::deployment_credentials::selection::{
    SelectedDeployment, credential_deployment, deployment_by_id,
};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::deployment_identity::stable_id::generate_model_id;
use betterlite_router::wildcard_catalog::Candidate;
use serde::Serialize;

use super::error::Failure;
use super::input::{Function, Input, Row};
use super::warnings::Warnings;

// The answer a case compares: the deployment the lookup selected, its credentials, and the warnings the call logged.
#[derive(Serialize)]
pub(crate) struct Answer {
    id: Option<String>,
    credentials: Option<JsonObject>,
    warnings: Vec<String>,
}

// The production call the case named, each answer read from the arguments it needs.
pub(crate) fn answer(input: &Input, captured: &Warnings) -> Result<Answer, Failure> {
    let catalog = catalog_rows(input)?;
    let model_id = input
        .arguments
        .model_id
        .as_deref()
        .ok_or(Failure::Argument("model_id"))?;
    let (id, credentials) = match input.function {
        // get_deployment then get_deployment_credentials: the id map only, and no named credential resolution.
        Function::GetDeploymentCredentials => {
            let id = selected_id(deployment_by_id(&catalog, model_id)?.as_ref());
            let credentials =
                deployment_credentials(&catalog, model_id)?.map(|params| params.fields);
            (id, credentials)
        }
        // get_credential_deployment then get_deployment_credentials_with_provider, over the case's credential_list.
        Function::CredentialsWithProvider => {
            let team = input.arguments.team_id.as_deref();
            let id = selected_id(credential_deployment(&catalog, model_id, team)?.as_ref());
            let credentials = credential_snapshot(input);
            let credentials = credentials_with_provider(&catalog, model_id, team, &credentials)?
                .map(|params| params.fields);
            (id, credentials)
        }
    };
    Ok(Answer {
        id,
        credentials,
        warnings: captured.messages(),
    })
}

// The id of the deployment a call selected, when it selected one.
fn selected_id(selected: Option<&SelectedDeployment>) -> Option<String> {
    selected
        .and_then(|chosen| chosen.deployment.id())
        .map(str::to_owned)
}

// set_model_list: every row as a deployment under the catalog's indices, then the patterns a case adds raw.
fn catalog_rows(input: &Input) -> Result<DeploymentCatalog, Failure> {
    let mut catalog = DeploymentCatalog::default();
    for row in &input.model_list {
        let spec = spec(row)?;
        catalog.index_patterns(&spec)?;
        catalog.push(spec);
    }
    for row in &input.pattern_deployments {
        let spec = spec(&row.deployment)?;
        let model = match spec.litellm_params.get("model") {
            Some(Json::Text(model)) => model.clone(),
            _ => String::new(),
        };
        let candidate = Candidate {
            model_id: spec.id().map(str::to_owned),
            model,
        };
        catalog
            .pattern_router
            .add_pattern(&row.pattern, candidate)?;
        catalog
            .pattern_router
            .record_deployment(&row.pattern, &spec);
    }
    Ok(catalog)
}

// set_model_list's per-row identity: the declared id, or the stable one it derives from the name and params.
fn spec(row: &Row) -> Result<DeploymentSpec, Failure> {
    let mut model_info = row.model_info.clone();
    if !model_info.contains_key("id") {
        let generated = generate_model_id(&row.model_name, &row.litellm_params);
        model_info.insert("id".to_owned(), Json::Text(generated));
    }
    Ok(DeploymentSpec::new(
        row.model_name.clone(),
        row.litellm_params.clone(),
        model_info,
        JsonObject::new(),
        String::new,
    )?)
}

// litellm.credential_list as the case declares it.
fn credential_snapshot(input: &Input) -> CredentialSnapshot {
    let entries = input
        .credentials
        .iter()
        .map(|row| CredentialEntry {
            credential_name: row.credential_name.clone(),
            credential_values: row.credential_values.clone(),
        })
        .collect();
    CredentialSnapshot::new(entries)
}

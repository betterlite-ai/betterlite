use std::sync::PoisonError;

use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_server::team_model_cleanup::alias::delete_team_model_alias;
use betterlite_server::team_model_cleanup::deployments::{
    team_deployments, team_public_model_names,
};
use betterlite_server::team_model_cleanup::remove::{
    DeletedModel, RemovalContext, ServingNames, remove_unbacked_team_models,
};
use serde_json::{Value, json};

use super::support::{FakeCache, FakeServing, FakeStore};
use super::{AliasCall, Call, DeploymentCall, Input, RemoveCall, failure};

pub(super) async fn answer(input: &Input) -> HelperResult {
    let mut answers = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        answers.push(answer_call(call).await?);
    }
    Ok(json!({ "calls": answers }))
}

async fn answer_call(call: &Call) -> HelperResult {
    match call {
        Call::DeleteTeamModelAlias(alias_call) => alias_answer(alias_call).await,
        Call::GetTeamDeployments(deployment_call) => {
            deployment_answer(deployment_call, false).await
        }
        Call::GetTeamPublicModelNames(public_names_call) => {
            deployment_answer(public_names_call, true).await
        }
        Call::RemoveUnbackedTeamModels(remove_call) => remove_answer(remove_call).await,
    }
}

async fn alias_answer(call: &AliasCall) -> HelperResult {
    let store = FakeStore::new("", None, call.aliases.clone(), Vec::new());
    let removed = delete_team_model_alias(&call.public_model_name, &store)
        .await
        .map_err(|error| failure("CleanupError", error.to_string()))?;
    let pairs: Vec<Value> = removed
        .into_iter()
        .map(|(team_id, alias)| json!({ "team_id": team_id, "alias": alias }))
        .collect();
    Ok(json!({ "removed": pairs, "aliases": store.alias_summary() }))
}

async fn deployment_answer(call: &DeploymentCall, public_names: bool) -> HelperResult {
    let store = FakeStore::new(&call.team_id, None, Vec::new(), call.deployments.clone());
    if public_names {
        let found = team_public_model_names(&call.team_id, &store)
            .await
            .map_err(|error| failure("CleanupError", error.to_string()))?;
        let names: Vec<String> = found.into_iter().collect();
        return Ok(json!({ "names": names }));
    }
    let rows = team_deployments(&call.team_id, &store)
        .await
        .map_err(|error| failure("CleanupError", error.to_string()))?;
    let names: Vec<String> = rows.into_iter().map(|row| row.model_name).collect();
    Ok(json!({ "deployments": names }))
}

async fn remove_answer(call: &RemoveCall) -> HelperResult {
    let store = FakeStore::new(
        call.team_id.as_deref().unwrap_or_default(),
        call.team_id.as_ref().and_then(|_| call.team_models.clone()),
        call.aliases.clone(),
        call.deployments.clone(),
    );
    let cache = FakeCache::default();
    let router = call.served_names.as_deref().map(FakeServing::new);
    let serving = router.as_ref().map(|fake| -> &dyn ServingNames { fake });
    let model = DeletedModel {
        model_name: call.model_name.clone(),
        team_id: call.team_id.clone(),
        team_public_model_name: call.team_public_model_name.clone(),
    };
    remove_unbacked_team_models(
        &model,
        &RemovalContext {
            aliases: &store,
            deployments: &store,
            teams: &store,
            serving,
            cache: &cache,
        },
    )
    .await
    .map_err(|error| failure("CleanupError", error.to_string()))?;
    Ok(json!({
        "team_models": store
            .team_models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        "cache": cache.summary(),
        "aliases": store.alias_summary(),
    }))
}

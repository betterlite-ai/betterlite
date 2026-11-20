use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_server::team_model_cleanup::alias::delete_team_model_alias;
use betterlite_server::team_model_cleanup::deployments::{
    team_deployments, team_public_model_names,
};
use betterlite_server::team_model_cleanup::error::CleanupError;
use betterlite_server::team_model_cleanup::remove::{
    DeletedModel, ServingNames, remove_unbacked_team_models,
};
use serde_json::{Value, json};

use super::support::{FakeCache, FakeServing, FakeStore, context};
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
        Call::DeleteTeamModelAlias(call) => alias_answer(call).await,
        Call::GetTeamDeployments(call) => deployments_answer(call).await,
        Call::GetTeamPublicModelNames(call) => public_names_answer(call).await,
        Call::RemoveUnbackedTeamModels(call) => remove_answer(call).await,
    }
}

async fn alias_answer(call: &AliasCall) -> HelperResult {
    let store = FakeStore::new("", None, call.aliases.clone(), Vec::new());
    let removed = delete_team_model_alias(&call.public_model_name, &store)
        .await
        .map_err(cleanup_failure)?;
    let pairs: Vec<Value> = removed
        .into_iter()
        .map(|(team_id, alias)| json!({ "team_id": team_id, "alias": alias }))
        .collect();
    Ok(json!({ "removed": pairs, "aliases": store.alias_summary() }))
}

async fn deployments_answer(call: &DeploymentCall) -> HelperResult {
    let store = FakeStore::new(&call.team_id, None, Vec::new(), call.deployments.clone());
    let found = team_deployments(&call.team_id, &store)
        .await
        .map_err(cleanup_failure)?;
    let names: Vec<String> = found.into_iter().map(|row| row.model_name).collect();
    Ok(json!({ "deployments": names }))
}

async fn public_names_answer(call: &DeploymentCall) -> HelperResult {
    let store = FakeStore::new(&call.team_id, None, Vec::new(), call.deployments.clone());
    let found = team_public_model_names(&call.team_id, &store)
        .await
        .map_err(cleanup_failure)?;
    let names: Vec<String> = found.into_iter().collect();
    Ok(json!({ "names": names }))
}

async fn remove_answer(call: &RemoveCall) -> HelperResult {
    let store = FakeStore::new(
        call.team_id.as_deref().unwrap_or_default(),
        call.team_id.as_ref().and(call.team_models.clone()),
        call.aliases.clone(),
        call.deployments.clone(),
    );
    let cache = FakeCache::default();
    let router = call
        .served_names
        .as_ref()
        .map(|names| FakeServing::new(names));
    let serving = router.as_ref().map(|fake| fake as &dyn ServingNames);
    let model = DeletedModel {
        model_name: call.model_name.clone(),
        team_id: call.team_id.clone(),
        team_public_model_name: call.team_public_model_name.clone(),
    };
    remove_unbacked_team_models(&model, &context(&store, &cache, serving))
        .await
        .map_err(cleanup_failure)?;
    Ok(json!({
        "team_models": store.current_models(),
        "cache": cache.summary(),
        "aliases": store.alias_summary(),
    }))
}

fn cleanup_failure(error: CleanupError) -> Box<HelperError> {
    failure("CleanupError", error.to_string())
}

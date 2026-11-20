use betterlite_context::anthropic_catalog_lookup::supports_fast_mode;
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::reasoning_effort_capability::{
    deployment_is_catalog_mapped, resolve_supported_efforts,
};

use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::alias::AliasTarget;
use crate::model_catalog_lookup::error::LookupError;
use crate::model_group_info::error::GroupInfoError;
use crate::model_group_info::group::ModelGroupInfo;
use crate::model_group_info::row::{
    RowParams, configured_limits, fallback_metadata, first_group, merge_later,
};
use crate::model_group_info::sources::{
    DeploymentInfo, GroupSources, ProviderAnswer, ProviderFailure, ProviderRequest,
    ProviderResolver,
};
use crate::model_group_info::state::AggregateState;

// get_model_group_info: an alias follows to its target and answers under the alias name, a hidden one answers none.
pub async fn get_model_group_info<D, R, P>(
    sources: &GroupSources<'_, D, R, P>,
    model_group: &str,
    keys: &mut ModelCostKeys,
) -> Result<Option<ModelGroupInfo>, GroupInfoError>
where
    D: DeploymentInfo,
    R: ProviderResolver,
    P: SupportedParameterSource,
{
    let Some(target) = sources.lookup.model_group_alias.get(model_group) else {
        return set_model_group_info(sources, model_group, model_group, keys).await;
    };
    match target {
        AliasTarget::Name(target) => set_model_group_info(sources, target, model_group, keys).await,
        AliasTarget::Item { model, hidden } => {
            alias_item(sources, model.as_ref(), hidden.as_ref(), model_group, keys).await
        }
        AliasTarget::Unsupported(_) => Ok(None),
    }
}

async fn alias_item<D, R, P>(
    sources: &GroupSources<'_, D, R, P>,
    model: Option<&Json>,
    hidden: Option<&Json>,
    alias: &str,
    keys: &mut ModelCostKeys,
) -> Result<Option<ModelGroupInfo>, GroupInfoError>
where
    D: DeploymentInfo,
    R: ProviderResolver,
    P: SupportedParameterSource,
{
    let Some(hidden) = hidden else {
        return Err(malformed_alias(alias, "hidden"));
    };
    if matches!(hidden, Json::Bool(true)) {
        return Ok(None);
    }
    match model {
        None => Err(malformed_alias(alias, "model")),
        Some(Json::Null) => Ok(None),
        Some(Json::Text(target)) => set_model_group_info(sources, target, alias, keys).await,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(GroupInfoError::AliasModel {
                alias: alias.to_owned(),
            })
        }
    }
}

fn malformed_alias(alias: &str, field: &'static str) -> GroupInfoError {
    GroupInfoError::Lookup(LookupError::MalformedAlias {
        alias: alias.to_owned(),
        field,
    })
}

async fn set_model_group_info<D, R, P>(
    sources: &GroupSources<'_, D, R, P>,
    model_group: &str,
    user_facing: &str,
    keys: &mut ModelCostKeys,
) -> Result<Option<ModelGroupInfo>, GroupInfoError>
where
    D: DeploymentInfo,
    R: ProviderResolver,
    P: SupportedParameterSource,
{
    let rows = sources.lookup.get_model_list(Some(model_group), None)?;
    let pattern_matches = sources
        .lookup
        .catalog
        .pattern_router
        .route(model_group, None)
        .is_some();
    let mut state = AggregateState::new(model_group, user_facing);
    for row in &rows {
        if row.model_name != model_group && !pattern_matches {
            continue;
        }
        advance(sources, row, &mut state, keys).await?;
    }
    Ok(state.finish())
}

async fn advance<D, R, P>(
    sources: &GroupSources<'_, D, R, P>,
    row: &DeploymentSpec,
    state: &mut AggregateState<'_>,
    keys: &mut ModelCostKeys,
) -> Result<(), GroupInfoError>
where
    D: DeploymentInfo,
    R: ProviderResolver,
    P: SupportedParameterSource,
{
    let params = RowParams::parse(&row.litellm_params)?;
    let mut limits = configured_limits(row)?;
    let found = lookup_metadata(sources, row, &params, keys).await;
    let mapped = deployment_is_catalog_mapped(found.as_ref(), &row.model_info);
    let answer = provider_answer(sources, &params)?;
    state.auth = Some(params.clientside_auth);
    let metadata = resolve_metadata(sources, state.model_group, row, &answer, found)?;
    let mut current = match state.group.take() {
        Some(mut group) => {
            merge_later(&mut group, &answer, &metadata, &mut limits)?;
            group
        }
        None => first_group(state.user_facing, &answer, &metadata)?,
    };
    current.supports_fast_mode = current.supports_fast_mode
        && supports_fast_mode(
            sources.fast_mode_catalog,
            &answer.litellm_model,
            &answer.provider,
        )
        .map_err(|source| GroupInfoError::FastMode { source })?;
    state.reasoning.observe(resolve_supported_efforts(
        &metadata,
        mapped,
        sources.cost_map,
    ));
    state.totals.add(&limits)?;
    state.group = Some(current);
    Ok(())
}

// The metadata lookup: an id the row lost or a failing lookup both leave the fallback in place.
async fn lookup_metadata<D, R, P>(
    sources: &GroupSources<'_, D, R, P>,
    row: &DeploymentSpec,
    params: &RowParams<'_>,
    keys: &mut ModelCostKeys,
) -> Option<JsonObject>
where
    D: DeploymentInfo,
{
    let id = row.id()?;
    sources
        .deployment_info
        .info(id, params.model, keys)
        .await
        .ok()
        .flatten()
}

fn provider_answer<D, R, P>(
    sources: &GroupSources<'_, D, R, P>,
    params: &RowParams<'_>,
) -> Result<ProviderAnswer, GroupInfoError>
where
    R: ProviderResolver,
{
    let request = ProviderRequest {
        model: params.model,
        custom_llm_provider: params.custom_llm_provider,
    };
    match sources.providers.resolve_provider(&request) {
        Ok(answer) => Ok(answer),
        Err(ProviderFailure::BadRequest(message)) => {
            tracing::error!("litellm.router.py::get_model_group_info() - {message}");
            Ok(ProviderAnswer {
                litellm_model: String::new(),
                provider: String::new(),
            })
        }
        Err(ProviderFailure::Other(message)) => Err(GroupInfoError::Provider { message }),
    }
}

// The deployment metadata, or the ModelMapInfo the fallback builds out of the row's own model_info.
fn resolve_metadata<D, R, P>(
    sources: &GroupSources<'_, D, R, P>,
    model_group: &str,
    row: &DeploymentSpec,
    answer: &ProviderAnswer,
    found: Option<JsonObject>,
) -> Result<JsonObject, GroupInfoError>
where
    P: SupportedParameterSource,
{
    let Some(metadata) = found else {
        let supported = supported_params(sources.parameters, answer)?;
        return fallback_metadata(model_group, &answer.provider, row, supported);
    };
    Ok(metadata)
}

fn supported_params<P: SupportedParameterSource>(
    parameters: &P,
    answer: &ProviderAnswer,
) -> Result<Vec<String>, GroupInfoError> {
    let provider = Some(answer.provider.as_str());
    match parameters.supported_openai_params(&answer.litellm_model, provider) {
        Ok(Some(params)) => Ok(params),
        Ok(None) => Ok(Vec::new()),
        Err(error) => Err(GroupInfoError::Parameters {
            message: error.to_string(),
        }),
    }
}

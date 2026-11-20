use std::future::Future;

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
use crate::model_catalog_lookup::lookup::CatalogLookup;
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

// The catalog half of get_model_group_info: the resolved group, its stored rows and whether a wildcard route covers it.
pub struct GroupRows {
    pub model_group: String,
    pub rows: Vec<DeploymentSpec>,
    pub pattern_matches: bool,
}

// get_model_group_info: an alias follows to its target and answers under the alias name, a hidden one answers none.
pub fn get_model_group_info<'call, D, R, P>(
    lookup: &CatalogLookup<'_>,
    sources: &'call GroupSources<'call, D, R, P>,
    model_group: &'call str,
    keys: &'call mut ModelCostKeys,
) -> impl Future<Output = Result<Option<ModelGroupInfo>, GroupInfoError>> + Send + 'call
where
    D: DeploymentInfo + Sync,
    R: ProviderResolver + Sync,
    P: SupportedParameterSource,
{
    let prepared = group_rows(lookup, model_group);
    async move {
        let Some(rows) = prepared? else {
            return Ok(None);
        };
        set_model_group_info(sources, &rows, model_group, keys).await
    }
}

// get_model_list plus pattern_router.route: everything the aggregation reads from the catalog, before the first await.
fn group_rows(
    lookup: &CatalogLookup<'_>,
    model_group: &str,
) -> Result<Option<GroupRows>, GroupInfoError> {
    let target = match lookup.model_group_alias.get(model_group) {
        None => Some(model_group.to_owned()),
        Some(entry) => alias_target(entry, model_group)?,
    };
    let Some(resolved) = target else {
        return Ok(None);
    };
    let rows = lookup.get_model_list(Some(resolved.as_str()), None)?;
    let pattern_matches = lookup
        .catalog
        .pattern_router
        .route(&resolved, None)
        .is_some();
    Ok(Some(GroupRows {
        model_group: resolved,
        rows,
        pattern_matches,
    }))
}

// The alias reads: a string target, an item's model unless it is hidden, and nothing for an unsupported kind.
fn alias_target(target: &AliasTarget, alias: &str) -> Result<Option<String>, GroupInfoError> {
    match target {
        AliasTarget::Name(name) => Ok(Some(name.clone())),
        AliasTarget::Unsupported(_) => Ok(None),
        AliasTarget::Item { model, hidden } => {
            let Some(flag) = hidden else {
                return Err(malformed_alias(alias, "hidden"));
            };
            if matches!(flag, Json::Bool(true)) {
                return Ok(None);
            }
            match model {
                None => Err(malformed_alias(alias, "model")),
                Some(Json::Null) => Ok(None),
                Some(Json::Text(name)) => Ok(Some(name.clone())),
                Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                    Err(GroupInfoError::AliasModel {
                        alias: alias.to_owned(),
                    })
                }
            }
        }
    }
}

fn malformed_alias(alias: &str, field: &'static str) -> GroupInfoError {
    GroupInfoError::Lookup(LookupError::MalformedAlias {
        alias: alias.to_owned(),
        field,
    })
}

// The aggregation half. Its future holds no catalog borrow, and that is what keeps it Send.
async fn set_model_group_info<D, R, P>(
    sources: &GroupSources<'_, D, R, P>,
    rows: &GroupRows,
    user_facing: &str,
    keys: &mut ModelCostKeys,
) -> Result<Option<ModelGroupInfo>, GroupInfoError>
where
    D: DeploymentInfo + Sync,
    R: ProviderResolver + Sync,
    P: SupportedParameterSource,
{
    let mut state = AggregateState::new(&rows.model_group, user_facing);
    for row in &rows.rows {
        // _set_model_group_info's match: the row's own name, or a group name the wildcard router covers.
        let matched = row.model_name == rows.model_group || rows.pattern_matches;
        if !matched {
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
    D: DeploymentInfo + Sync,
    R: ProviderResolver + Sync,
    P: SupportedParameterSource,
{
    let params = RowParams::parse(&row.litellm_params)?;
    let mut limits = configured_limits(row)?;
    let found = lookup_metadata(sources.deployment_info, row, &params, keys).await;
    let mapped = deployment_is_catalog_mapped(found.as_ref(), &row.model_info);
    let answer = provider_answer(sources.providers, &params)?;
    state.auth = params.clientside_auth;
    let metadata = if let Some(metadata) = found {
        metadata
    } else {
        let supported = supported_params(sources.parameters, &answer)?;
        fallback_metadata(state.model_group, &answer.provider, row, &supported)?
    };
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
        .map_err(GroupInfoError::from)?;
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
async fn lookup_metadata<D>(
    deployment_info: &D,
    row: &DeploymentSpec,
    params: &RowParams<'_>,
    keys: &mut ModelCostKeys,
) -> Option<JsonObject>
where
    D: DeploymentInfo + Sync,
{
    let id = row.id()?;
    deployment_info
        .info(id, params.model, keys)
        .await
        .ok()
        .flatten()
}

fn provider_answer<R>(
    providers: &R,
    params: &RowParams<'_>,
) -> Result<ProviderAnswer, GroupInfoError>
where
    R: ProviderResolver,
{
    let request = ProviderRequest {
        model: params.model,
        custom_llm_provider: params.custom_llm_provider,
    };
    match providers.resolve_provider(&request) {
        Ok(answer) => Ok(answer),
        Err(ProviderFailure::BadRequest(message)) => {
            tracing::error!("litellm.router.py::get_model_group_info() - {message}");
            Ok(ProviderAnswer::default())
        }
        Err(ProviderFailure::Other(message)) => Err(GroupInfoError::Provider { message }),
    }
}

fn supported_params<P>(
    parameters: &P,
    answer: &ProviderAnswer,
) -> Result<Vec<String>, GroupInfoError>
where
    P: SupportedParameterSource,
{
    let provider = Some(answer.provider.as_str());
    parameters
        .supported_openai_params(&answer.litellm_model, provider)
        .map(Option::unwrap_or_default)
        .map_err(|error| GroupInfoError::Parameters {
            message: error.to_string(),
        })
}

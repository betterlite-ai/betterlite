use std::collections::BTreeSet;

use betterlite_auth::auth_model_lists::access_groups::AccessGroupScope;
use betterlite_auth::auth_model_lists::complete_list::{
    ListingFlags, ListingRequest, get_complete_model_list,
};
use betterlite_auth::auth_model_lists::key_models::get_key_models;
use betterlite_auth::auth_model_lists::lists::dedup_first;
use betterlite_auth::auth_model_lists::team_models::get_team_models;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use crate::model_listing::ModelListingError;
use crate::model_listing::access::{
    AccessLookups, KeyModelFields, ListingSources, RouterCatalog, TeamListing, model_names,
};

// get_available_models_for_user's arguments, with LiteLLM's defaults for the flags and the two optional names.
#[derive(Clone, Copy, Debug, Default)]
pub struct AvailableModelsRequest<'request> {
    pub user_model: Option<&'request str>,
    pub team_id: Option<&'request str>,
    pub flags: ListingFlags,
}

// The caller's list before wildcard expansion: both grant kinds, the access group models and the team it names.
struct ListingGrant<'grant> {
    key_models: Vec<String>,
    team_models: Vec<String>,
    access_group_models: Vec<String>,
    effective_team_id: Option<&'grant str>,
}

pub async fn available_models_for_user(
    key: &UserAPIKeyAuth,
    request: &AvailableModelsRequest<'_>,
    sources: &ListingSources<'_>,
) -> Result<Vec<String>, ModelListingError> {
    let fields = KeyModelFields::from_key(key)?;
    let catalog = RouterCatalog::read(sources.router);
    let grant = listing_grant(key, *request, &fields, &catalog, sources.access).await?;
    let all_models = complete_list(&grant, request, &catalog, sources)?;
    let Some(agent_visible) =
        agent_visible_models(&fields, request, &grant, &catalog, sources).await?
    else {
        return Ok(all_models);
    };
    Ok(all_models
        .into_iter()
        .filter(|model| agent_visible.contains(model))
        .collect())
}

// The caller's grants before wildcard expansion: the validated team, the two lists and the group models.
async fn listing_grant<'grant>(
    key: &UserAPIKeyAuth,
    request: AvailableModelsRequest<'grant>,
    fields: &'grant KeyModelFields,
    catalog: &RouterCatalog,
    access: &dyn AccessLookups,
) -> Result<ListingGrant<'grant>, ModelListingError> {
    let requested = requested_team(key, request.team_id, access).await?;
    let key_grants = key_models(fields, requested.as_ref(), request.flags.groups, catalog);
    // the validated requested team's models stand in for the key's stored team models
    let team_source = requested.as_ref().map_or_else(
        || model_names(&fields.team_models),
        |team| team.models.clone(),
    );
    let team_grants = get_team_models(
        &team_source,
        &catalog.model_names,
        &catalog.access_groups,
        request.flags.groups.include,
    );
    let effective_team_id = request.team_id.or(fields.team_id.as_deref());
    let group_models = if key_grants.is_empty() && team_grants.is_empty() {
        Vec::new()
    } else {
        access_group_models(key, requested.as_ref(), effective_team_id, access).await
    };
    Ok(ListingGrant {
        key_models: key_grants,
        team_models: team_grants,
        access_group_models: group_models,
        effective_team_id,
    })
}

// _get_validated_team_object: the requested team, which stands in for the key's own grants only for its members.
async fn requested_team(
    key: &UserAPIKeyAuth,
    team_id: Option<&str>,
    access: &dyn AccessLookups,
) -> Result<Option<TeamListing>, ModelListingError> {
    let Some(requested_id) = team_id else {
        return Ok(None);
    };
    match access.team(requested_id).await {
        Ok(Some(team)) => {
            access.validate_membership(key, &team).await?;
            Ok(Some(team))
        }
        Ok(None) => Ok(None),
        Err(missing) => Err(ModelListingError::TeamNotFound(missing)),
    }
}

// get_key_models: the key's own grants, and nothing once a validated requested team replaces them.
fn key_models(
    fields: &KeyModelFields,
    requested_team: Option<&TeamListing>,
    scope: AccessGroupScope,
    catalog: &RouterCatalog,
) -> Vec<String> {
    if requested_team.is_some() {
        return Vec::new();
    }
    get_key_models(
        &model_names(&fields.models),
        &model_names(&fields.team_models),
        &catalog.model_names,
        &catalog.access_groups,
        scope,
    )
}

// _get_access_group_models: the team's access group models first, then the key's own, first-seen order.
async fn access_group_models(
    key: &UserAPIKeyAuth,
    requested_team: Option<&TeamListing>,
    effective_team_id: Option<&str>,
    access: &dyn AccessLookups,
) -> Vec<String> {
    let fetched_team = match requested_team {
        Some(_) => None,
        None => team_for_access_groups(effective_team_id, access).await,
    };
    let team = requested_team.or(fetched_team.as_ref());
    let team_group_models = access.team_access_group_models(team).await;
    let key_group_models = access.key_access_group_models(key, team).await;
    let combined: Vec<String> = team_group_models
        .into_iter()
        .chain(key_group_models)
        .collect();
    dedup_first(combined)
}

// _get_team_object_for_access_groups: a team the listing cannot fetch leaves the key's own groups in place.
async fn team_for_access_groups(
    team_id: Option<&str>,
    access: &dyn AccessLookups,
) -> Option<TeamListing> {
    let lookup_id = team_id?;
    match access.team(lookup_id).await {
        Ok(team) => team,
        Err(missing) => {
            tracing::debug!(
                team_id = %missing.team_id,
                "Could not fetch team while listing models"
            );
            None
        }
    }
}

// the complete list: each grant with its access group models appended only when it already restricts the caller
fn complete_list(
    grant: &ListingGrant<'_>,
    request: &AvailableModelsRequest<'_>,
    catalog: &RouterCatalog,
    sources: &ListingSources<'_>,
) -> Result<Vec<String>, ModelListingError> {
    let granted_key_models = with_access_groups(&grant.key_models, &grant.access_group_models);
    let granted_team_models = with_access_groups(&grant.team_models, &grant.access_group_models);
    let listing = ListingRequest {
        key_models: &granted_key_models,
        team_models: &granted_team_models,
        user_model: request.user_model,
        team_id: grant.effective_team_id,
        model_access_groups: &catalog.access_groups,
        flags: request.flags,
    };
    let models =
        get_complete_model_list(&catalog.model_names, &listing, &sources.expansion_context())?;
    Ok(models)
}

fn with_access_groups(models: &[String], group_models: &[String]) -> Vec<String> {
    if models.is_empty() {
        return Vec::new();
    }
    models.iter().chain(group_models).cloned().collect()
}

// _agent_access_group_visible_models: the models an agent key's ceiling still grants, None when nothing caps it.
async fn agent_visible_models(
    fields: &KeyModelFields,
    request: &AvailableModelsRequest<'_>,
    grant: &ListingGrant<'_>,
    catalog: &RouterCatalog,
    sources: &ListingSources<'_>,
) -> Result<Option<BTreeSet<String>>, ModelListingError> {
    let Some(agent_id) = fields.agent_id.as_deref() else {
        return Ok(None);
    };
    let Some(ceiling) = sources.access.agent_ceiling(agent_id).await else {
        return Ok(None);
    };
    if sources.router.is_none() {
        return Ok(Some(ceiling));
    }
    let granted = get_team_models(
        &ceiling.into_iter().collect::<Vec<String>>(),
        &catalog.model_names,
        &catalog.access_groups,
        request.flags.groups.include,
    );
    if granted.is_empty() {
        return Ok(Some(BTreeSet::new()));
    }
    let visible = ceiling_list(&granted, request, grant.effective_team_id, catalog, sources)?;
    Ok(Some(visible.into_iter().collect()))
}

// the ceiling's list: its granted models expanded like a key grant, with no team models and no only-groups filter
fn ceiling_list(
    granted: &[String],
    request: &AvailableModelsRequest<'_>,
    team_id: Option<&str>,
    catalog: &RouterCatalog,
    sources: &ListingSources<'_>,
) -> Result<Vec<String>, ModelListingError> {
    let listing = ListingRequest {
        key_models: granted,
        team_models: &[],
        user_model: None,
        team_id,
        model_access_groups: &catalog.access_groups,
        flags: ListingFlags {
            infer_model_from_keys: false,
            return_wildcard_routes: request.flags.return_wildcard_routes,
            groups: AccessGroupScope {
                include: request.flags.groups.include,
                only: false,
            },
        },
    };
    let models =
        get_complete_model_list(&catalog.model_names, &listing, &sources.expansion_context())?;
    Ok(models)
}

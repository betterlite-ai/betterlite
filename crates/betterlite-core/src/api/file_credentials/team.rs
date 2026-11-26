use std::error::Error;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_model_lists::access_groups::AccessGroupScope;
use betterlite_auth::auth_model_lists::complete_list::{
    ListingFlags, ListingRequest, get_complete_model_list,
};
use betterlite_auth::auth_model_lists::key_models::get_key_models;
use betterlite_auth::auth_model_lists::lists::dedup_first;
use betterlite_auth::auth_model_lists::wildcard::{ExpansionContext, WildcardError};
use betterlite_auth::auth_model_patterns::{ALL_PROXY_MODELS, ALL_TEAM_MODELS};
use betterlite_auth::entity_model_access::grant::text_models;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use serde::Deserialize;
use thiserror::Error;

use crate::api::file_credentials::catalog::CredentialCatalog;
use crate::api::model_credentials::{CredentialSource, prepare_data_with_credentials};

// The UserAPIKeyAuth fields the provider search reads; a caller with no key, such as a JWT user, has none.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct KeyGrants {
    models: Vec<JsonData>,
    team_models: Vec<JsonData>,
    team_id: Option<String>,
}

// What get_team_provider_credentials reads once and both of its searches share.
struct TeamScope {
    team_id: Option<String>,
    team_models: Vec<String>,
    proxy_model_list: Vec<String>,
    model_access_groups: IndexMap<String, Vec<String>>,
    key_allowlist: Vec<String>,
}

#[derive(Debug, Error)]
pub enum TeamCredentialError<SourceError>
where
    SourceError: Error + 'static,
{
    #[error("the caller's key grants could not be read")]
    KeyGrants(#[from] serde_json::Error),
    #[error("the credential catalog failed to read its {read}")]
    Catalog {
        read: &'static str,
        #[source]
        source: SourceError,
    },
    #[error("the credential catalog failed to resolve model `{model_id}`")]
    Credentials {
        model_id: String,
        #[source]
        source: SourceError,
    },
    #[error(transparent)]
    Expansion(#[from] WildcardError),
}

fn key_grants(principal: &Principal) -> Result<KeyGrants, serde_json::Error> {
    let Some(key) = principal.source_key.0.as_ref() else {
        return Ok(KeyGrants::default());
    };
    serde_json::from_value(serde_json::to_value(key)?)
}

// get_key_models over a restricted key; an empty list is unrestrained and a sentinel defers to the team.
fn expanded_key_allowlist(
    raw_key_models: &[String],
    team_models: &[String],
    proxy_model_list: &[String],
    model_access_groups: &IndexMap<String, Vec<String>>,
) -> Vec<String> {
    let defers_to_team = raw_key_models
        .iter()
        .any(|model| model == ALL_PROXY_MODELS || model == ALL_TEAM_MODELS);
    if raw_key_models.is_empty() || defers_to_team {
        return Vec::new();
    }
    get_key_models(
        raw_key_models,
        team_models,
        proxy_model_list,
        model_access_groups,
        AccessGroupScope::default(),
    )
}

fn team_scope<Catalog>(
    catalog: &Catalog,
    principal: &Principal,
) -> Result<TeamScope, TeamCredentialError<Catalog::Error>>
where
    Catalog: CredentialCatalog,
{
    let grants = key_grants(principal)?;
    let team_models = text_models(&grants.team_models);
    let proxy_model_list = catalog
        .model_names(grants.team_id.as_deref())
        .map_err(|source| TeamCredentialError::Catalog {
            read: "model names",
            source,
        })?;
    let model_access_groups =
        catalog
            .model_access_groups()
            .map_err(|source| TeamCredentialError::Catalog {
                read: "model access groups",
                source,
            })?;
    let key_allowlist = expanded_key_allowlist(
        &text_models(&grants.models),
        &team_models,
        &proxy_model_list,
        &model_access_groups,
    );
    Ok(TeamScope {
        team_id: grants.team_id,
        team_models,
        proxy_model_list,
        model_access_groups,
        key_allowlist,
    })
}

// _key_may_use: an empty allowlist restrains nothing, and a restricted key needs a listed name.
fn may_use(scope: &TeamScope, public_model_name: &str) -> bool {
    scope.key_allowlist.is_empty()
        || scope
            .key_allowlist
            .iter()
            .any(|allowed| allowed == public_model_name)
}

// _provider_credentials: this provider's credentials without their model field, read as the caller's team.
fn provider_credentials<Catalog>(
    catalog: &Catalog,
    model_id: &str,
    team_id: Option<&str>,
    provider: &str,
) -> Result<Option<JsonObject>, TeamCredentialError<Catalog::Error>>
where
    Catalog: CredentialSource,
{
    let Some(credentials) = catalog
        .credentials_with_provider(model_id, team_id)
        .map_err(|source| TeamCredentialError::Credentials {
            model_id: model_id.to_owned(),
            source,
        })?
    else {
        return Ok(None);
    };
    if !matches!(
        credentials.get("custom_llm_provider"),
        Some(Json::Text(found)) if found.as_str() == provider
    ) {
        return Ok(None);
    }
    let without_model = credentials
        .into_iter()
        .filter(|(name, _)| name != "model")
        .collect();
    Ok(Some(without_model))
}

// 1. the team's own BYOK row: the first that names this team, carries an id and a usable public name.
fn byok_credentials<Catalog>(
    catalog: &Catalog,
    scope: &TeamScope,
    provider: &str,
) -> Result<Option<JsonObject>, TeamCredentialError<Catalog::Error>>
where
    Catalog: CredentialCatalog,
{
    let Some(team) = scope.team_id.as_deref() else {
        return Ok(None);
    };
    let deployments = catalog
        .deployments()
        .map_err(|source| TeamCredentialError::Catalog {
            read: "deployments",
            source,
        })?;
    for deployment in deployments {
        if deployment.info_text("team_id") != Some(team) {
            continue;
        }
        let Some(deployment_id) = deployment.info_text("id") else {
            continue;
        };
        let configured_name = deployment
            .info_text("team_public_model_name")
            .filter(|name| !name.is_empty());
        let public_name = configured_name.unwrap_or(&deployment.model_name);
        if !may_use(scope, public_name) {
            continue;
        }
        if let Some(credentials) =
            provider_credentials(catalog, deployment_id, Some(team), provider)?
        {
            return Ok(Some(credentials));
        }
    }
    Ok(None)
}

// 2. the deployments the caller may use; all-proxy-models on the team defers to the team-scoped list.
fn fallback_credentials<Catalog>(
    catalog: &Catalog,
    scope: &TeamScope,
    provider: &str,
    expansion: &ExpansionContext<'_>,
) -> Result<Option<JsonObject>, TeamCredentialError<Catalog::Error>>
where
    Catalog: CredentialCatalog,
{
    let team_grants_all = scope
        .team_models
        .iter()
        .any(|model| model == ALL_PROXY_MODELS);
    let effective_team_models: &[String] = if team_grants_all {
        &[]
    } else {
        &scope.team_models
    };
    let listing = ListingRequest {
        key_models: &scope.key_allowlist,
        team_models: effective_team_models,
        user_model: None,
        team_id: scope.team_id.as_deref(),
        model_access_groups: &scope.model_access_groups,
        flags: ListingFlags {
            infer_model_from_keys: false,
            return_wildcard_routes: true,
            groups: AccessGroupScope {
                include: true,
                only: false,
            },
        },
    };
    let models_to_try = get_complete_model_list(&scope.proxy_model_list, &listing, expansion)?;
    for model in dedup_first(models_to_try) {
        if let Some(credentials) =
            provider_credentials(catalog, &model, scope.team_id.as_deref(), provider)?
        {
            return Ok(Some(credentials));
        }
    }
    Ok(None)
}

// get_team_provider_credentials: the team's BYOK row first, then the caller's deployments in B759 order.
pub fn get_team_provider_credentials<Catalog>(
    catalog: Option<&Catalog>,
    principal: &Principal,
    provider: &str,
    expansion: &ExpansionContext<'_>,
) -> Result<Option<JsonObject>, TeamCredentialError<Catalog::Error>>
where
    Catalog: CredentialCatalog,
{
    let Some(catalog) = catalog else {
        return Ok(None);
    };
    let scope = team_scope(catalog, principal)?;
    if let Some(credentials) = byok_credentials(catalog, &scope, provider)? {
        return Ok(Some(credentials));
    }
    fallback_credentials(catalog, &scope, provider, expansion)
}

// apply_team_provider_credentials: a miss leaves data untouched, so the caller keeps its own fallback.
pub fn apply_team_provider_credentials<Catalog>(
    data: &mut JsonObject,
    catalog: Option<&Catalog>,
    principal: &Principal,
    provider: &str,
    expansion: &ExpansionContext<'_>,
) -> Result<(), TeamCredentialError<Catalog::Error>>
where
    Catalog: CredentialCatalog,
{
    let Some(credentials) = get_team_provider_credentials(catalog, principal, provider, expansion)?
    else {
        return Ok(());
    };
    // include_internal_credentials is false, as in the source
    drop(prepare_data_with_credentials(
        data,
        &credentials,
        None,
        false,
    ));
    Ok(())
}

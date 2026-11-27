use betterlite_core::api::file_credentials::catalog::CredentialCatalog;
use betterlite_core::api::file_credentials::routing::{ModelGrant, authorize_model_for_key};
use betterlite_core::api::model_credentials::{
    DEFAULT_OPERATION_CONTEXT, get_credentials_for_model, prepare_data_with_credentials,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::error::VectorFileResolutionError;
use super::{
    Caller, KeyFacts, RequestFacts, RoutingDeps, has_content, header_text, last_query_value,
};

// What _update_request_data_with_model_routing_hint's first half left the route: hint routing settled it, or the team fallback runs next.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelHintStep {
    // api_key or api_base was set, including "", so every hint is ignored
    Bypassed,
    // the hinted deployment's credentials were applied
    CredentialsApplied,
    // no hint routing applied credentials, so team_model_fallback must run
    FallbackRequired,
}

// The hint half: api_key/api_base bypass, body before query before header, authorization and credential application.
pub async fn model_hint_precedence<Catalog, Grants>(
    data: &mut JsonObject,
    facts: &RequestFacts<'_>,
    caller: Option<&Caller<'_>>,
    deps: &RoutingDeps<'_, Catalog, Grants>,
) -> Result<ModelHintStep, VectorFileResolutionError<Catalog::Error, Grants::Error>>
where
    Catalog: CredentialCatalog + Sync,
    Grants: ModelGrant<Catalog> + Sync,
{
    if data
        .get("api_key")
        .is_some_and(|value| !matches!(value, Json::Null))
        || data
            .get("api_base")
            .is_some_and(|value| !matches!(value, Json::Null))
    {
        return Ok(ModelHintStep::Bypassed);
    }
    let user_controlled = user_controlled_hint(facts);
    let Some(hint) = model_hint(data, user_controlled.as_deref()) else {
        return Ok(ModelHintStep::FallbackRequired);
    };
    let should_authorize = user_controlled.as_deref() == Some(hint.as_str());
    let Some(credentials) = hint_credentials(deps, caller, &hint, should_authorize).await? else {
        return Ok(ModelHintStep::FallbackRequired);
    };
    let _snapshot = prepare_data_with_credentials(data, &credentials, None, false);
    Ok(ModelHintStep::CredentialsApplied)
}

// The wildcard branch passes the caller's team and only routes on credentials; the exact branch always looks the name up.
async fn hint_credentials<Catalog, Grants>(
    deps: &RoutingDeps<'_, Catalog, Grants>,
    caller: Option<&Caller<'_>>,
    hint: &str,
    should_authorize: bool,
) -> Result<Option<JsonObject>, VectorFileResolutionError<Catalog::Error, Grants::Error>>
where
    Catalog: CredentialCatalog + Sync,
    Grants: ModelGrant<Catalog> + Sync,
{
    if hint.contains('*') {
        let Some(catalog) = deps.catalog else {
            return Ok(None);
        };
        if should_authorize {
            authorize_hint(Some(catalog), deps.grants, caller, hint).await?;
        }
        let team_id = match caller {
            Some(caller) => {
                let key: KeyFacts = serde_json::to_value(caller.auth)
                    .and_then(serde_json::from_value)
                    .map_err(VectorFileResolutionError::Caller)?;
                key.team_id
            }
            None => None,
        };
        return catalog
            .credentials_with_provider(hint, team_id.as_deref())
            .map_err(|source| VectorFileResolutionError::Catalog {
                model_id: hint.to_owned(),
                source,
            });
    }
    if should_authorize {
        authorize_hint(deps.catalog, deps.grants, caller, hint).await?;
    }
    let credentials = get_credentials_for_model(deps.catalog, hint, DEFAULT_OPERATION_CONTEXT)?;
    Ok(Some(credentials))
}

// _authorize_model_routing_hint: no auth makes the grant a no-op.
async fn authorize_hint<Catalog, Grants>(
    catalog: Option<&Catalog>,
    grants: &Grants,
    caller: Option<&Caller<'_>>,
    model: &str,
) -> Result<(), VectorFileResolutionError<Catalog::Error, Grants::Error>>
where
    Catalog: CredentialCatalog + Sync,
    Grants: ModelGrant<Catalog> + Sync,
{
    let Some(caller) = caller else {
        return Ok(());
    };
    authorize_model_for_key(catalog, grants, model, caller.principal)
        .await
        .map_err(VectorFileResolutionError::Authorization)
}

// request.query_params.get("model") or request.headers.get("x-litellm-model"): the query's last value, then the header.
fn user_controlled_hint(facts: &RequestFacts<'_>) -> Option<String> {
    match last_query_value(facts.query, "model") {
        Some(model) if !model.is_empty() => Some(model),
        Some(_) | None => header_text(facts.headers, "x-litellm-model"),
    }
}

// data.get("model") or the caller's hint; a truthy body model that is not text is not a hint at all.
fn model_hint(data: &JsonObject, user_controlled: Option<&str>) -> Option<String> {
    match data.get("model") {
        Some(value) if has_content(value) => match value {
            Json::Text(text) => Some(text.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        },
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => user_controlled.map(str::to_owned),
    }
}

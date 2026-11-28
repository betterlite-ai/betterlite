use std::str::FromStr as _;

use betterlite_core::api::file_credentials::catalog::CredentialCatalog;
use betterlite_core::api::file_credentials::routing::{ModelGrant, authorize_model_for_key};
use betterlite_core::api::model_credentials::prepare_data_with_credentials;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_registration::JsonObject;

use super::error::VectorFileResolutionError;
use super::{Caller, KeyFacts, RoutingDeps, has_content};

// The team model list entries that name every model rather than one deployment.
const SENTINELS: [&str; 3] = ["all-team-models", "all-proxy-models", "no-default-models"];

// The fallback half of _update_request_data_with_model_routing_hint: one OpenAI team deployment applies, two decline to choose.
pub async fn team_model_fallback<Catalog, Grants>(
    data: &mut JsonObject,
    caller: Option<&Caller<'_>>,
    deps: &RoutingDeps<'_, Catalog, Grants>,
) -> Result<(), VectorFileResolutionError<Catalog::Error, Grants::Error>>
where
    Catalog: CredentialCatalog + Sync,
    Grants: ModelGrant<Catalog> + Sync,
{
    let Some(present) = caller else {
        return Ok(());
    };
    let Some(catalog) = deps.catalog else {
        return Ok(());
    };
    let key: KeyFacts = serde_json::to_value(present.auth)
        .and_then(serde_json::from_value)
        .map_err(VectorFileResolutionError::Caller)?;
    let Some(candidates) = candidate_names(key.team_models.as_ref()) else {
        return Ok(());
    };
    let found = openai_team_credentials(
        catalog,
        deps.grants,
        present,
        &candidates,
        key.team_id.as_deref(),
    )
    .await?;
    if let Some(credentials) = found {
        let _snapshot = prepare_data_with_credentials(data, &credentials, None, false);
        return Ok(());
    }
    if let [name] = candidates.as_slice() {
        authorize_team_model(catalog, deps.grants, present, name).await?;
        data.insert("model".to_owned(), Json::Text(name.clone()));
    }
    Ok(())
}

// The loop authorizes every OpenAI match, so a second one returns without a choice and without looking further.
async fn openai_team_credentials<Catalog, Grants>(
    catalog: &Catalog,
    grants: &Grants,
    caller: &Caller<'_>,
    candidates: &[String],
    team_id: Option<&str>,
) -> Result<Option<JsonObject>, VectorFileResolutionError<Catalog::Error, Grants::Error>>
where
    Catalog: CredentialCatalog + Sync,
    Grants: ModelGrant<Catalog> + Sync,
{
    let mut openai_credentials = None;
    for model_name in candidates {
        let Some(found) = catalog
            .credentials_with_provider(model_name, team_id)
            .map_err(|source| VectorFileResolutionError::Catalog {
                model_id: model_name.clone(),
                source,
            })?
        else {
            continue;
        };
        if !is_openai(&found) {
            continue;
        }
        authorize_team_model(catalog, grants, caller, model_name).await?;
        if openai_credentials.is_some() {
            return Ok(None);
        }
        openai_credentials = Some(found);
    }
    Ok(openai_credentials)
}

// _authorize_model_routing_hint over a team deployment name, with the real caller in hand.
async fn authorize_team_model<Catalog, Grants>(
    catalog: &Catalog,
    grants: &Grants,
    caller: &Caller<'_>,
    model: &str,
) -> Result<(), VectorFileResolutionError<Catalog::Error, Grants::Error>>
where
    Catalog: CredentialCatalog + Sync,
    Grants: ModelGrant<Catalog> + Sync,
{
    authorize_model_for_key(Some(catalog), grants, model, caller.principal)
        .await
        .map_err(VectorFileResolutionError::Authorization)
}

// team_models with the sentinels and every non-string entry dropped; None stops the fallback for a truthy non-list.
pub(super) fn candidate_names(team_models: Option<&Json>) -> Option<Vec<String>> {
    let Some(value) = team_models.filter(|value| has_content(value)) else {
        return Some(Vec::new());
    };
    let Json::List(entries) = value else {
        return None;
    };
    let mut names = Vec::new();
    for entry in entries {
        let Json::Text(name) = entry else {
            continue;
        };
        if SENTINELS.contains(&name.as_str()) {
            continue;
        }
        names.push(name.clone());
    }
    Some(names)
}

// provider != LlmProviders.OPENAI.value after the model-prefix inference that only a missing provider allows.
fn is_openai(credentials: &JsonObject) -> bool {
    let provider = match credentials.get("custom_llm_provider") {
        Some(Json::Text(provider)) => Some(provider.as_str()),
        Some(Json::Null) | None => match credentials.get("model") {
            Some(Json::Text(model)) => model.split_once('/').map(|(prefix, _rest)| prefix),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        },
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => None,
    };
    provider.and_then(|name| ProviderId::from_str(name).ok()) == Some(ProviderId::Openai)
}

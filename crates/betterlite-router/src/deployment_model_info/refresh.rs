use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, ResolvedModel, resolve};
use betterlite_openai::compatible_models::{
    MODEL_INFO_DISCOVERY_PROVIDERS, MODEL_INFO_REFRESH_CONCURRENCY, ModelListCache, ModelQuery,
    TokenLimits, discover_model_limits,
};
use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret as _, SecretString};

use crate::catalog_updates::catalog::{CatalogHandle, DeploymentCatalog};
use crate::deployment_credentials::resolve::credentials_with_provider;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::cache::CacheClear;

use super::cache::DiscoveryCache;
use super::error::RefreshFailure;
use super::types::DiscoveredDeploymentModelInfo;
use super::value::holds;

// Everything Router.arefresh_model_info reads: the published catalog, the caches it fills and B174's resolution inputs.
pub struct RefreshContext<'call, 'resolution> {
    pub catalogs: &'call CatalogHandle,
    pub discovery: &'call DiscoveryCache,
    pub credentials: &'call CredentialSnapshot,
    pub resolution: &'call ResolutionContext<'resolution>,
    pub invalidation: &'call (dyn CacheClear + Sync),
    pub client: &'call ProxiedClient,
    pub model_list_cache: &'call dyn ModelListCache,
}

// Router.arefresh_model_info: one snapshot, eight workers pulling it, then the model-group metadata invalidation.
pub async fn refresh_model_info(context: &RefreshContext<'_, '_>) {
    let snapshot = context.catalogs.load_full();
    let cursor = AtomicUsize::new(0);
    let workers = std::array::from_fn::<_, MODEL_INFO_REFRESH_CONCURRENCY, _>(|_| {
        refresh_worker(context, &snapshot, &cursor)
    });
    let [w0, w1, w2, w3, w4, w5, w6, w7] = workers;
    tokio::join!(w0, w1, w2, w3, w4, w5, w6, w7);
    context.invalidation.clear();
}

// One worker of arefresh_model_info.refresh_worker: it pulls until the snapshot is spent and reports each failure with the same debug line.
async fn refresh_worker(
    context: &RefreshContext<'_, '_>,
    snapshot: &Arc<DeploymentCatalog>,
    cursor: &AtomicUsize,
) {
    loop {
        let index = cursor.fetch_add(1, Ordering::Relaxed);
        let Some(deployment) = snapshot.models.get(index) else {
            return;
        };
        if refresh_deployment(context, snapshot, deployment, index)
            .await
            .is_err()
        {
            tracing::debug!("Could not refresh deployment model info");
        }
    }
}

// Router._arefresh_deployment_model_info for one row: the skip checks, the discovery and the guarded write.
async fn refresh_deployment(
    context: &RefreshContext<'_, '_>,
    snapshot: &Arc<DeploymentCatalog>,
    deployment: &DeploymentSpec,
    index: usize,
) -> Result<(), RefreshFailure> {
    let params = merged_params(snapshot, deployment, context.credentials)?;
    let resolved = resolve_deployment(&params, context.resolution)?;
    if !MODEL_INFO_DISCOVERY_PROVIDERS.contains(&resolved.provider.slug()) {
        return Ok(());
    }
    let Some(api_base) = resolved.api_base.as_deref() else {
        return Ok(());
    };
    if resolved.model.contains('*') {
        return Ok(());
    }
    if params.get("use_clientside_credentials").is_some_and(holds) {
        return Ok(());
    }
    let api_key = params_api_key(&params)?.or_else(|| resolved.api_key.clone());
    let headers = discovery_headers(&params, api_key.as_ref())?;
    let query = ModelQuery {
        model: &resolved.model,
        api_base,
        headers: &headers,
    };
    let limits = discover_model_limits(context.client, context.model_list_cache, &query)
        .await
        .unwrap_or_default();
    write_discovery(context, snapshot, deployment, index, limits);
    Ok(())
}

// The overlay of B2306's credentials on the deployment params, with the params' null fields left out.
fn merged_params(
    catalog: &DeploymentCatalog,
    deployment: &DeploymentSpec,
    credentials: &CredentialSnapshot,
) -> Result<JsonObject, RefreshFailure> {
    let mut params: JsonObject = deployment
        .litellm_params
        .iter()
        .filter(|(_, value)| !matches!(value, Json::Null))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let overlay = credentials_with_provider(
        catalog,
        deployment.id().unwrap_or_default(),
        None,
        credentials,
    );
    if let Some(overlay) = overlay.map_err(RefreshFailure::Credentials)? {
        params.extend(overlay.fields);
    }
    Ok(params)
}

fn optional_text<'params>(
    params: &'params JsonObject,
    field: &str,
) -> Result<Option<&'params str>, RefreshFailure> {
    match params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(RefreshFailure::Params)
        }
    }
}

// get_llm_provider(model=params.model, litellm_params=params) through B174's resolver.
fn resolve_deployment(
    params: &JsonObject,
    resolution: &ResolutionContext<'_>,
) -> Result<ResolvedModel, RefreshFailure> {
    let Some(Json::Text(model)) = params.get("model") else {
        return Err(RefreshFailure::Params);
    };
    let custom_llm_provider = optional_text(params, "custom_llm_provider")?;
    let api_base = optional_text(params, "api_base")?;
    let api_key = optional_text(params, "api_key")?.map(SecretString::from);
    let request = ResolutionRequest {
        model,
        custom_llm_provider,
        api_base,
        api_key: api_key.as_ref(),
    };
    resolve(&request, resolution).map_err(RefreshFailure::Resolution)
}

// params.api_key as LiteLLM_Params types it: a non-empty string, with anything else not text a validation failure.
fn params_api_key(params: &JsonObject) -> Result<Option<SecretString>, RefreshFailure> {
    match params.get("api_key") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(key)) if key.is_empty() => Ok(None),
        Some(Json::Text(key)) => Ok(Some(SecretString::from(key.clone()))),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(RefreshFailure::Params)
        }
    }
}

// The authorization header first, then the truthy extra_headers or headers lowercased over it, so an explicit Authorization wins.
fn discovery_headers(
    params: &JsonObject,
    api_key: Option<&SecretString>,
) -> Result<Headers, RefreshFailure> {
    let mut headers = Headers::new();
    if let Some(key) = api_key {
        let bearer = key.expose_secret();
        headers.insert("authorization".to_owned(), format!("Bearer {bearer}"));
    }
    let chosen = match params.get("extra_headers").is_some_and(holds) {
        true => params.get("extra_headers"),
        false => params.get("headers").filter(|value| holds(value)),
    };
    let Some(chosen) = chosen else {
        return Ok(headers);
    };
    let Json::Object(fields) = chosen else {
        return Err(RefreshFailure::Headers);
    };
    for (name, value) in fields {
        let Json::Text(value) = value else {
            return Err(RefreshFailure::Headers);
        };
        headers.insert(name.to_lowercase(), value.clone());
    }
    Ok(headers)
}

// The write guard: empty limits, a missing id and a replaced catalog or row all leave the cache untouched.
fn write_discovery(
    context: &RefreshContext<'_, '_>,
    snapshot: &Arc<DeploymentCatalog>,
    deployment: &DeploymentSpec,
    index: usize,
    limits: TokenLimits,
) {
    if limits == TokenLimits::default() {
        return;
    }
    let Some(model_id) = deployment.id() else {
        return;
    };
    let record = DiscoveredDeploymentModelInfo::new(Arc::clone(snapshot), index, limits);
    let live = context.catalogs.load_full();
    if !record.is_current(&live, model_id) {
        return;
    }
    context.discovery.resize(live.models.len());
    context.discovery.delete(model_id);
    context.discovery.store(model_id, record);
    context.invalidation.clear();
}

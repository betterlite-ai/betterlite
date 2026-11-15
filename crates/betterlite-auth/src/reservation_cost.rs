use std::collections::BTreeSet;
use std::fmt::Debug;

use async_trait::async_trait;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use rust_decimal::Decimal;

pub mod error;

mod cost_info;
mod estimates;
mod rates;
mod tokens;

#[cfg(test)]
mod tests;

use crate::auth_request_models::RequestedModels;
use crate::auth_request_models::catalog::{
    CatalogError, Collaborators, ManagedIdDecoders, ModelCatalog,
};
use crate::auth_request_models::model::get_model_from_request;
use crate::auth_request_models::sources::{DispatchContext, RequestSnapshot};

// betterlite-auth cannot depend on betterlite-router, so the router's cached metadata arrives through this port.
pub trait RouterModelInfo: Debug + Send + Sync {
    fn get_model_list(
        &self,
        model_name: &str,
    ) -> Result<Vec<RouterDeployment>, error::ModelInfoError>;

    fn cached_deployment_model_info(
        &self,
        model_id: &str,
        backend_model: &str,
    ) -> Result<Option<JsonObject>, error::ModelInfoError>;

    fn cached_model_group_info(
        &self,
        model_name: &str,
    ) -> Result<Option<JsonObject>, error::ModelInfoError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterDeployment {
    pub model_id: Option<String>,
    pub backend_model: Option<String>,
}

// The fallback for a router-less admission: litellm.get_model_info, which B2225 implements over its catalog.
pub trait ModelInfoSource: Debug + Send + Sync {
    fn get_model_info(&self, model: &str) -> Result<Option<JsonObject>, error::ModelInfoError>;
}

// B1695 owns the tokenizer dispatch; this port only says which requests and models to count.
#[async_trait]
pub trait ReservationTokens: Debug + Send + Sync {
    async fn count_input_tokens(
        &self,
        body: &JsonObject,
        models: &[String],
    ) -> IndexMap<String, usize>;

    async fn count_input_tokens_for_model(&self, body: &JsonObject, model: &str) -> Option<usize>;
}

pub struct EstimateRequest<'request> {
    pub body: &'request JsonObject,
    pub route: &'request str,
}

pub struct EstimateSources<'sources> {
    pub router: Option<&'sources dyn RouterModelInfo>,
    pub model_info: &'sources dyn ModelInfoSource,
    pub tokens: &'sources dyn ReservationTokens,
    pub catalog: Option<&'sources dyn ModelCatalog>,
    pub decoders: &'sources dyn ManagedIdDecoders,
}

pub async fn estimate_request_max_cost(
    request: &EstimateRequest<'_>,
    sources: &EstimateSources<'_>,
    input_token_counts: Option<&IndexMap<String, usize>>,
) -> Result<Option<Decimal>, error::ReservationCostError> {
    let models = request_models(request, sources)?;
    let mut best: Option<Decimal> = None;
    for model in models {
        let counted =
            tokens::counted_input_tokens(request.body, sources.tokens, &model, input_token_counts)
                .await;
        let infos = cost_info::load(&model, sources.router, sources.model_info)?;
        for info in &infos {
            if let Some(cost) =
                estimates::max_cost_for_info(request.body, request.route, counted, info)?
            {
                best = Some(best.map_or(cost, |current| current.max(cost)));
            }
        }
    }
    Ok(best)
}

pub async fn estimate_request_input_cost(
    request: &EstimateRequest<'_>,
    sources: &EstimateSources<'_>,
    input_token_counts: Option<&IndexMap<String, usize>>,
) -> Result<Option<Decimal>, error::ReservationCostError> {
    let models = request_models(request, sources)?;
    let mut best: Option<Decimal> = None;
    for model in models {
        let counted =
            tokens::counted_input_tokens(request.body, sources.tokens, &model, input_token_counts)
                .await;
        let infos = cost_info::load(&model, sources.router, sources.model_info)?;
        for info in &infos {
            if let Some(cost) = estimates::input_cost_for_info(counted, info)? {
                best = Some(best.map_or(cost, |current| current.max(cost)));
            }
        }
    }
    Ok(best)
}

pub async fn count_request_input_tokens(
    request: &EstimateRequest<'_>,
    sources: &EstimateSources<'_>,
) -> Result<IndexMap<String, usize>, error::ReservationCostError> {
    let models = request_models(request, sources)?;
    if models.is_empty() {
        return Ok(IndexMap::new());
    }
    Ok(sources
        .tokens
        .count_input_tokens(request.body, &models)
        .await)
}

fn request_models(
    request: &EstimateRequest<'_>,
    sources: &EstimateSources<'_>,
) -> Result<Vec<String>, CatalogError> {
    let path_params = BTreeSet::new();
    let snapshot = RequestSnapshot {
        route: request.route,
        body: request.body,
        headers: None,
        query: None,
        dispatch: DispatchContext {
            endpoint_marker: None,
            path_params: &path_params,
        },
        team_id: None,
    };
    let collaborators = Collaborators {
        catalog: sources.catalog,
        decoders: sources.decoders,
    };
    Ok(match get_model_from_request(&snapshot, &collaborators)? {
        RequestedModels::None => Vec::new(),
        RequestedModels::One(model) => vec![model],
        RequestedModels::Many(models) => models,
    })
}

use crate::error_body::ModelError;
use crate::model_query::read_query;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use axum::{Json, Router};
use betterlite_config::types::ModelDeployment;
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::Arc;
mod tests;
// LiteLLM's DEFAULT_MODEL_CREATED_AT_TIME
const MODEL_CREATED_AT: u64 = 1_677_610_602;
// in the order of LiteLLM's model_list signature, the order its validation errors follow
const LIST_FLAGS: [&str; 5] = [
    "return_wildcard_routes",
    "include_model_access_groups",
    "only_model_access_groups",
    "include_metadata",
    "healthy_only",
];
struct ModelGroup {
    name: String,
    first_model: Option<String>,
}
#[derive(Serialize)]
struct ModelObject<'group> {
    id: &'group str,
    object: &'static str,
    created: u64,
    owned_by: &'group str,
}
#[derive(Serialize)]
struct ModelListBody<'group> {
    data: Vec<ModelObject<'group>>,
    object: &'static str,
}
pub(crate) fn model_routes(deployments: &[ModelDeployment]) -> Router {
    let mut seen = BTreeSet::new();
    let mut groups = Vec::new();
    for deployment in deployments {
        if seen.insert(&deployment.model_name) {
            let first_model = deployment.litellm_params.model.clone();
            groups.push(ModelGroup {
                name: deployment.model_name.clone(),
                first_model,
            });
        }
    }
    Router::new()
        .route("/models", get(list_models))
        .route("/v1/models", get(list_models))
        .route("/models/{model_id}", get(retrieve_model))
        .route("/v1/models/{model_id}", get(retrieve_model))
        .with_state(Arc::<[ModelGroup]>::from(groups))
}
async fn list_models(
    State(groups): State<Arc<[ModelGroup]>>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> Result<Response, ModelError> {
    // the other flags only act on wildcards, access groups and health checks, which the config can't hold yet
    let (values, [_, _, only_access_groups, include_metadata, _]) =
        read_query(raw_query.as_deref(), LIST_FLAGS)?;
    if let Some(scope) = values.get("scope").filter(|scope| *scope != "expand") {
        let detail = format!("scope must be expand, got {scope}");
        return Err(ModelError::Detail(StatusCode::BAD_REQUEST, detail));
    }
    if include_metadata {
        return Err(ModelError::Unsupported("include_metadata".to_owned()));
    }
    if headers.contains_key("anthropic-version") {
        let feature = "the Anthropic model list".to_owned();
        return Err(ModelError::Unsupported(feature));
    }
    // with no access groups in the config, a list of only access groups is empty
    let listed: Vec<&ModelGroup> = groups.iter().filter(|_| !only_access_groups).collect();
    if let Some(group) = listed.iter().find(|group| group.name.contains('*')) {
        let feature = format!("the wildcard model {}", group.name);
        return Err(ModelError::Unsupported(feature));
    }
    // LiteLLM lists every model as owned by openai, whatever its provider
    let data = listed
        .iter()
        .map(|group| ModelObject {
            id: &group.name,
            object: "model",
            created: MODEL_CREATED_AT,
            owned_by: "openai",
        })
        .collect();
    Ok(Json(ModelListBody {
        data,
        object: "list",
    })
    .into_response())
}
async fn retrieve_model(
    State(groups): State<Arc<[ModelGroup]>>,
    Path(model_id): Path<String>,
    RawQuery(raw_query): RawQuery,
) -> Result<Response, ModelError> {
    // health checks that could hide a model are not in the config yet
    read_query(raw_query.as_deref(), ["healthy_only"])?;
    let Some(group) = groups.iter().find(|group| group.name == model_id) else {
        let detail = format!("model {model_id} is not configured");
        return Err(ModelError::Detail(StatusCode::NOT_FOUND, detail));
    };
    let model = group.first_model.as_deref().unwrap_or_default();
    let Some((provider, _)) = model.split_once('/') else {
        let feature = format!("the model {model} without a provider prefix");
        return Err(ModelError::Unsupported(feature));
    };
    let body = ModelObject {
        id: &group.name,
        object: "model",
        created: MODEL_CREATED_AT,
        owned_by: provider,
    };
    Ok(Json(body).into_response())
}

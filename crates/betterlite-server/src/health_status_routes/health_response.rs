use std::collections::BTreeSet;

use axum::Json;
use axum::http::header::HeaderName;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse as _, Response};
use betterlite_models::decisions::Json as Decisions;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

use crate::health_access_and_connections::result_filter::strip_admin_only_fields;
use crate::health_access_and_connections::visibility::HealthCaller;
use crate::health_probe_settings::display::ADMIN_ONLY_DISPLAY_PARAMS;
use crate::health_status_routes::error::HealthRouteError;
use crate::health_status_routes::query::HealthQuery;

const NOTICE_HEADER: &str = "litellm-health-field-notice";
const WARNING_TEXT: &str = "Some accessible deployments are missing model_info.id and were excluded from this response. Ask a proxy admin to populate model_info.id for these models.";

// _post_process's two switches: whether the caller named a model, and whether it is a proxy admin
#[derive(Clone, Copy)]
pub(super) struct PostProcess {
    pub(super) model_specific: bool,
    pub(super) admin: bool,
}

// _post_process: the 503 for a healthy-less targeted answer, then the admin-only fields and their notice header
pub(super) fn post_process(
    result: JsonObject,
    mode: PostProcess,
) -> Result<Response, HealthRouteError> {
    let status = if mode.model_specific && healthy_count(&result) == 0 {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    if mode.admin {
        return Ok((status, Json(result)).into_response());
    }
    let body = strip_admin_only_fields(&result);
    let notice = notice_header()?;
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert(HeaderName::from_static(NOTICE_HEADER), notice);
    Ok(response)
}

// result.get("healthy_count", 0): a result without the key counts as none
fn healthy_count(result: &JsonObject) -> u64 {
    let Some(Decisions::Number(number)) = result.get("healthy_count") else {
        return 0;
    };
    number.as_u64().unwrap_or(0)
}

fn notice_header() -> Result<HeaderValue, HealthRouteError> {
    let fields = ADMIN_ONLY_DISPLAY_PARAMS.join(", ");
    let value = format!("{fields} are admin-only on this endpoint");
    HeaderValue::from_str(&value).map_err(HealthRouteError::Notice)
}

pub(super) fn model_specific_request(query: &HealthQuery) -> bool {
    query
        .model
        .as_deref()
        .is_some_and(|value| !value.is_empty())
        || query
            .model_id
            .as_deref()
            .is_some_and(|value| !value.is_empty())
}

pub(super) fn deployment_ids(deployments: &[DeploymentSpec]) -> BTreeSet<String> {
    deployments
        .iter()
        .filter_map(DeploymentSpec::id)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn warn_missing_ids(caller: &HealthCaller) {
    let user_id = caller.user_id.as_deref().unwrap_or("None");
    let models = caller.models.join(", ");
    tracing::warn!(
        "health_endpoint: scoped key {user_id} has accessible models [{models}] but none of the matching deployments carry a model_info.id; background health-check cache will return an empty result."
    );
}

pub(super) fn add_missing_id_warning(result: &mut JsonObject) {
    result.insert(
        "warnings".to_owned(),
        Decisions::List(vec![Decisions::Text(WARNING_TEXT.to_owned())]),
    );
}

use axum::http::StatusCode;

use crate::health_access_and_connections::visibility::HealthRouter;
use crate::health_status_routes::error::HealthRouteError;

// _health_endpoint_resolve_target_model_name: model_id wins over model, and an id no deployment carries is a 404
pub fn resolve_target_model_name(
    model: Option<&str>,
    model_id: Option<&str>,
    router: Option<&dyn HealthRouter>,
) -> Result<Option<String>, HealthRouteError> {
    let Some(model_id) = model_id.filter(|id| !id.is_empty()) else {
        return Ok(model.map(str::to_owned));
    };
    let Some(router) = router else {
        return Err(not_found(model_id));
    };
    match router.get_deployment(model_id) {
        Ok(Some(deployment)) => Ok(Some(deployment.model_name)),
        Ok(None) => Err(not_found(model_id)),
        Err(error) => {
            tracing::error!("Error getting deployment for model_id {model_id}: {error}");
            Err(not_found(model_id))
        }
    }
}

fn not_found(model_id: &str) -> HealthRouteError {
    HealthRouteError::http(
        StatusCode::NOT_FOUND,
        format!("Model with ID {model_id} not found"),
    )
}

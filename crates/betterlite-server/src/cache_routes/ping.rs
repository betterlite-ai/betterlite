use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_cache::types::ping::{CachePingResponse, HealthCheckCacheParams};
use serde::Serialize;

use super::CacheRoutesState;
use super::admin::CacheAdminError;

const PING_ERROR_TYPE: &str = "cache_ping_error";
const PING_ERROR_PARAM: &str = "cache_ping";
const UNINITIALIZED_PING_MESSAGE: &str = concat!(
    r#"{"message": "Cache not initialized. litellm.cache is None", "#,
    r#""litellm_cache_params": "{}", "health_check_cache_params": "{}"}"#
);
const UNHEALTHY_MESSAGE: &str = "Service Unhealthy";

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorObject,
}

#[derive(Serialize)]
struct ErrorObject {
    message: String,
    #[serde(rename = "type")]
    error_type: &'static str,
    param: &'static str,
    code: &'static str,
}

pub async fn cache_ping(State(state): State<CacheRoutesState>) -> Response {
    let Some(cache) = state.cache.current() else {
        return ping_failure(UNINITIALIZED_PING_MESSAGE.to_owned());
    };
    let params = cache.masked_cache_params();
    let health = cache.health_check_params();
    let kind = cache.cache_type();
    if kind != "redis" {
        let body = CachePingResponse {
            status: "healthy".to_owned(),
            cache_type: kind,
            ping_response: None,
            set_cache_response: None,
            litellm_cache_params: Some(params),
            health_check_cache_params: None,
        };
        return (StatusCode::OK, Json(body)).into_response();
    }
    match cache.ping().await {
        Ok(pinged) => {
            let _written = cache.add_test_key().await;
            let body = CachePingResponse {
                status: "healthy".to_owned(),
                cache_type: kind,
                ping_response: Some(pinged),
                set_cache_response: Some("success".to_owned()),
                litellm_cache_params: Some(params),
                health_check_cache_params: Some(health),
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(source) => ping_unhealthy(&params, &health, &source),
    }
}

fn ping_unhealthy(
    params: &str,
    health: &HealthCheckCacheParams,
    source: &CacheAdminError,
) -> Response {
    // LiteLLM dumps {} when the params could not be extracted, which is also the only way this serialization fails
    let health_text = serde_json::to_string(health).unwrap_or_else(|_| "{}".to_owned());
    let message = format!(
        r#"{{"message": "{UNHEALTHY_MESSAGE}", "litellm_cache_params": {}, "health_check_cache_params": {}}}"#,
        serde_json::to_string(params).unwrap_or_default(),
        serde_json::to_string(&health_text).unwrap_or_default()
    );
    tracing::error!(error = %source, "Cache health check failed");
    ping_failure(message)
}

fn ping_failure(message: String) -> Response {
    let body = ErrorEnvelope {
        error: ErrorObject {
            message,
            error_type: PING_ERROR_TYPE,
            param: PING_ERROR_PARAM,
            code: "503",
        },
    };
    (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
}

mod tests;

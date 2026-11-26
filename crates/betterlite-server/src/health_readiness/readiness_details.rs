use async_trait::async_trait;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_config::sections::aiohttp_transport_toggle::Settings as AiohttpSettings;
use betterlite_config::sections::env_credential_login::Settings as EnvLoginSettings;
use serde::Serialize;
use thiserror::Error;

use crate::cache_routes::wire::unhealthy;
use crate::env_credential_login::is_enabled;
use crate::health_readiness::db_readiness::DbReadiness;

mod tests;

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(transparent)]
pub struct WireFlag(pub bool);

// One report per kind: the cache type alone, or a semantic cache with its index answer
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum CacheReport {
    Plain(String),
    Semantic {
        #[serde(rename = "type")]
        cache_type: String,
        index_info: String,
    },
}

#[derive(Debug, Error)]
pub enum ReadinessSourceError {
    #[error("{message}")]
    Unavailable { message: String },
}

#[async_trait]
pub trait ReadinessSources: Send + Sync {
    fn success_callback_names(&self) -> Vec<String>;
    async fn cache_report(&self) -> Result<Option<CacheReport>, ReadinessSourceError>;
    fn redis_configured(&self) -> bool;
    fn sso_provider_ready(&self) -> bool;
    fn log_level(&self) -> String;
    fn version(&self) -> String;
}

#[derive(Serialize)]
struct DetailsBody {
    status: &'static str,
    db: &'static str,
    cache: Option<CacheReport>,
    litellm_version: String,
    success_callbacks: Vec<String>,
    use_aiohttp_transport: WireFlag,
    log_level: String,
    is_detailed_debug: WireFlag,
    show_no_redis_warning: WireFlag,
    #[serde(rename = "show_env_credential_login_warning")]
    env_login_warning: WireFlag,
}

pub async fn respond(
    db: &DbReadiness,
    sources: &dyn ReadinessSources,
    env_login: &EnvLoginSettings,
    aiohttp: &AiohttpSettings,
) -> Response {
    match details(db, sources, env_login, aiohttp).await {
        Ok((status, body)) => (status, Json(body)).into_response(),
        Err(error) => unhealthy(&error.to_string()),
    }
}

async fn details(
    db: &DbReadiness,
    sources: &dyn ReadinessSources,
    env_login: &EnvLoginSettings,
    aiohttp: &AiohttpSettings,
) -> Result<(StatusCode, DetailsBody), ReadinessSourceError> {
    let success_callbacks = sources.success_callback_names();
    let cache = sources.cache_report().await?;
    // Python's isEnabledFor(DEBUG) is true at DEBUG and below, and NOTSET is 0
    let log_level = sources.log_level();
    let debug_enabled =
        log_level.eq_ignore_ascii_case("DEBUG") || log_level.eq_ignore_ascii_case("NOTSET");
    let show_no_redis_warning = db.show_no_redis_warning(sources).await;
    let env_login_warning = is_enabled(env_login, sources.sso_provider_ready());
    let report = db.resolve_public_db().await;
    let status = if report.unavailable {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    let body = DetailsBody {
        status: "healthy",
        db: report.status.as_str(),
        cache,
        litellm_version: sources.version(),
        success_callbacks,
        use_aiohttp_transport: WireFlag(!aiohttp.disable_aiohttp_transport),
        log_level,
        is_detailed_debug: WireFlag(debug_enabled),
        show_no_redis_warning: WireFlag(show_no_redis_warning),
        env_login_warning: WireFlag(env_login_warning),
    };
    Ok((status, body))
}

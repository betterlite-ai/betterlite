use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get, options};
use betterlite_config::sections::aiohttp_transport_toggle::Settings as AiohttpSettings;
use betterlite_config::sections::env_credential_login::Settings as EnvLoginSettings;
use betterlite_config::sections::health_readiness::Settings as HealthSettings;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use subtle::ConstantTimeEq as _;

use crate::admission_control::AdmissionControl;
use crate::health_readiness::callback_listing::{self, CallbackNameSource};
use crate::health_readiness::db_readiness::DbReadiness;
use crate::health_readiness::readiness_details::{self, ReadinessSources};
use crate::health_readiness::shutdown_state::{InFlightCounter, ShutdownState};

mod tests;

const DRAIN_TOKEN_HEADER: &str = "x-drain-token";
const ALLOWED_METHODS: &str = "GET, OPTIONS";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AdmissionCounters {
    pub admitted: u64,
    pub queued: u64,
    pub rejected: u64,
}

pub trait AdmissionCounter: Send + Sync {
    fn counters(&self) -> AdmissionCounters;
}

impl AdmissionCounter for AdmissionControl {
    fn counters(&self) -> AdmissionCounters {
        let stats = self.stats();
        AdmissionCounters {
            admitted: stats.admitted,
            queued: stats.queued,
            rejected: stats.rejected_total,
        }
    }
}

// The ports B8540 fills with production adapters; the in-flight one is shared with the drain wait
#[derive(Clone)]
pub struct ReadinessPorts {
    pub in_flight: Arc<dyn InFlightCounter>,
    pub admission: Arc<dyn AdmissionCounter>,
    pub db: Arc<DbReadiness>,
    pub sources: Arc<dyn ReadinessSources>,
    pub callbacks: Arc<dyn CallbackNameSource>,
}

#[derive(Clone)]
pub struct ReadinessState {
    pub shutdown: Arc<ShutdownState>,
    pub in_flight: Arc<dyn InFlightCounter>,
    pub admission: Arc<dyn AdmissionCounter>,
    pub db: Arc<DbReadiness>,
    pub sources: Arc<dyn ReadinessSources>,
    pub callbacks: Arc<dyn CallbackNameSource>,
    pub settings: HealthSettings,
    pub env_credential_login: EnvLoginSettings,
    pub aiohttp_transport: AiohttpSettings,
}

impl ReadinessState {
    #[must_use]
    pub fn new(
        ports: ReadinessPorts,
        settings: HealthSettings,
        env_credential_login: EnvLoginSettings,
        aiohttp_transport: AiohttpSettings,
    ) -> Self {
        let shutdown = Arc::new(ShutdownState::new(
            Arc::clone(&ports.in_flight),
            settings.graceful_shutdown_timeout,
        ));
        Self {
            shutdown,
            in_flight: ports.in_flight,
            admission: ports.admission,
            db: ports.db,
            sources: ports.sources,
            callbacks: ports.callbacks,
            settings,
            env_credential_login,
            aiohttp_transport,
        }
    }
}

// B8540 mounts this router behind the proxy's auth middleware, which exempts these paths
#[must_use]
pub fn readiness_routes(state: ReadinessState) -> Router {
    Router::new()
        .route(
            "/health/readiness",
            get(public_readiness).options(health_options),
        )
        .route("/health/readiness/details", get(details_route))
        .route("/health/backlog", get(health_backlog))
        .route("/health/drain", get(health_drain))
        .route("/health/license", get(health_license))
        .route("/health/liveliness", options(health_options))
        .route("/health/liveness", options(health_options))
        .route("/settings", get(active_callbacks))
        .route("/active/callbacks", get(active_callbacks))
        .with_state(state)
}

#[derive(Serialize)]
struct ShutdownBody {
    status: &'static str,
}

#[derive(Serialize)]
struct PublicReadinessBody {
    status: &'static str,
    db: &'static str,
}

#[derive(Serialize)]
struct BacklogBody {
    in_flight_requests: u64,
    admitted_requests: u64,
    queued_requests: u64,
    rejected_requests: u64,
}

#[derive(Serialize)]
struct DrainBody {
    status: &'static str,
    drained_requests: u64,
}

#[derive(Serialize)]
struct DetailBody {
    detail: &'static str,
}

#[derive(Serialize)]
struct LicenseBody {
    has_license: bool,
    license_type: &'static str,
    expiration_date: Option<&'static str>,
    allowed_features: Vec<&'static str>,
    limits: LicenseLimits,
}

#[derive(Serialize)]
struct LicenseLimits {
    max_users: Option<u32>,
    max_teams: Option<u32>,
}

async fn public_readiness(State(state): State<ReadinessState>) -> Response {
    if state.shutdown.is_shutting_down() {
        let body = ShutdownBody {
            status: "shutting_down",
        };
        return (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response();
    }
    if state.settings.allow_public_details {
        return readiness_details::respond(
            &state.db,
            &*state.sources,
            &state.env_credential_login,
            &state.aiohttp_transport,
        )
        .await;
    }
    let report = state.db.resolve_public_db().await;
    let body = PublicReadinessBody {
        status: "healthy",
        db: report.status.as_str(),
    };
    let status = if report.unavailable {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    (status, Json(body)).into_response()
}

async fn details_route(State(state): State<ReadinessState>) -> Response {
    readiness_details::respond(
        &state.db,
        &*state.sources,
        &state.env_credential_login,
        &state.aiohttp_transport,
    )
    .await
}

async fn health_backlog(State(state): State<ReadinessState>) -> Json<BacklogBody> {
    let counters = state.admission.counters();
    let in_flight_requests = state.in_flight.in_flight_requests();
    Json(BacklogBody {
        in_flight_requests,
        admitted_requests: counters.admitted,
        queued_requests: counters.queued,
        rejected_requests: counters.rejected,
    })
}

async fn health_drain(State(state): State<ReadinessState>, headers: HeaderMap) -> Response {
    if !state.settings.drain.enabled {
        let body = DetailBody {
            detail: "Not Found",
        };
        return (StatusCode::NOT_FOUND, Json(body)).into_response();
    }
    if let Some(expected) = state.settings.drain.token.as_ref()
        && !drain_token_matches(headers, expected)
    {
        let body = DetailBody {
            detail: "Invalid or missing X-Drain-Token",
        };
        return (StatusCode::UNAUTHORIZED, Json(body)).into_response();
    }
    state.shutdown.start_shutdown();
    let drained_requests = state.shutdown.wait_for_drain(None, true).await;
    let body = DrainBody {
        status: "drained",
        drained_requests,
    };
    Json(body).into_response()
}

// LiteLLM reads the first header value and compares it with compare_digest; an absent or
// non-UTF-8 header is the empty string, which no configured token is
fn drain_token_matches(mut headers: HeaderMap, expected: &SecretString) -> bool {
    let supplied_header = headers.remove(DRAIN_TOKEN_HEADER);
    let supplied = supplied_header
        .as_ref()
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    bool::from(
        supplied
            .as_bytes()
            .ct_eq(expected.expose_secret().as_bytes()),
    )
}

async fn health_license() -> Json<LicenseBody> {
    Json(LicenseBody {
        has_license: false,
        license_type: "community",
        expiration_date: None,
        allowed_features: Vec::new(),
        limits: LicenseLimits {
            max_users: None,
            max_teams: None,
        },
    })
}

async fn active_callbacks(
    State(state): State<ReadinessState>,
) -> Json<callback_listing::CallbackListingBody> {
    Json(callback_listing::callback_listing(
        &*state.callbacks,
        &state.settings,
    ))
}

async fn health_options() -> Response {
    let mut response = StatusCode::OK.into_response();
    let headers = response.headers_mut();
    headers.insert(header::ALLOW, HeaderValue::from_static(ALLOWED_METHODS));
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static(ALLOWED_METHODS),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("*"),
    );
    response
}

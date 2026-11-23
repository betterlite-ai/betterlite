pub mod beta;
pub mod beta_config;
pub mod beta_schedule;
pub mod beta_status;
pub mod caller;
pub mod cost_map;
pub mod cost_map_reads;
pub mod cost_map_schedule;
pub mod error;
pub mod hours;
pub mod ports;
pub mod wire;

mod tests;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use betterlite_models::source::CatalogClock;

use crate::periodic_reloads::state::PeriodicReloads;

use self::ports::{BetaHeadersReload, CatalogReload, ReloadConfigStore};

#[derive(Clone)]
pub struct ReloadRoutesState {
    // None is LiteLLM's prisma_client None: the cost map reload answers 500 outside its try, every other route wraps that 500
    pub config: Option<Arc<dyn ReloadConfigStore>>,
    pub catalog: Arc<dyn CatalogReload>,
    pub beta_headers: Arc<dyn BetaHeadersReload>,
    // the one B8385 state the timer checks write, so the status routes read this pod's own reload times
    pub reloads: Arc<PeriodicReloads>,
    pub clock: CatalogClock,
}

pub fn routes(state: ReloadRoutesState) -> Router {
    Router::new()
        .route(
            "/reload/model_cost_map",
            post(cost_map::reload_model_cost_map),
        )
        .route(
            "/schedule/model_cost_map_reload",
            post(cost_map_schedule::schedule_model_cost_map_reload)
                .delete(cost_map_schedule::cancel_model_cost_map_reload),
        )
        .route(
            "/schedule/model_cost_map_reload/status",
            get(cost_map_reads::cost_map_reload_status),
        )
        .route(
            "/model/cost_map/source",
            get(cost_map_reads::model_cost_map_source),
        )
        .route(
            "/reload/anthropic_beta_headers",
            post(beta::reload_anthropic_beta_headers),
        )
        .route(
            "/schedule/anthropic_beta_headers_reload",
            post(beta_schedule::schedule_beta_headers_reload)
                .delete(beta_schedule::cancel_beta_headers_reload),
        )
        .route(
            "/schedule/anthropic_beta_headers_reload/status",
            get(beta_status::anthropic_beta_headers_status),
        )
        .with_state(state)
}

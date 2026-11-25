use std::sync::Arc;

use betterlite_config::sections::health_probe_policy::Settings as ProbePolicy;
use betterlite_db::health_check_rows::HealthCheckRows;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::CatalogHandle;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysError, SysRng};
use serde_json::Number;
use tokio_util::task::TaskTracker;

use crate::health_access_and_connections::visibility::HealthRouter;
use crate::health_cycle::ports::ProviderHealth;
use crate::model_deprecation::clock::UtcClock;

// health_check_details: whether a probe's endpoint rows keep the approved detail fields
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetailVisibility {
    #[default]
    Hidden,
    Shown,
}

// use_background_health_checks: answer from the cache the background loop fills, or probe live
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HealthCheckSource {
    #[default]
    Live,
    Background,
}

// skip_disabled_models: leave a deployment that opted out in model_info out of a probe
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SkipDisabledModels {
    #[default]
    Keep,
    Skip,
}

// health_check_results as the background loop leaves it: the four keys GET /health answers from the cache
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HealthCheckResults {
    pub healthy_endpoints: Vec<JsonObject>,
    pub unhealthy_endpoints: Vec<JsonObject>,
    pub healthy_count: usize,
    pub unhealthy_count: usize,
}

impl HealthCheckResults {
    // _perform_health_check_and_save's return value, in LiteLLM's key order
    #[must_use]
    pub fn to_json(&self) -> JsonObject {
        let mut result = JsonObject::new();
        result.insert(
            "healthy_endpoints".to_owned(),
            Json::List(
                self.healthy_endpoints
                    .iter()
                    .cloned()
                    .map(Json::Object)
                    .collect(),
            ),
        );
        result.insert(
            "unhealthy_endpoints".to_owned(),
            Json::List(
                self.unhealthy_endpoints
                    .iter()
                    .cloned()
                    .map(Json::Object)
                    .collect(),
            ),
        );
        result.insert(
            "healthy_count".to_owned(),
            Json::Number(Number::from(self.healthy_count)),
        );
        result.insert(
            "unhealthy_count".to_owned(),
            Json::Number(Number::from(self.unhealthy_count)),
        );
        result
    }
}

// The cache the background loop fills; snapshot takes no lock of the server's and copies what it answers
pub trait HealthResultsSource: Send + Sync {
    fn snapshot(&self) -> HealthCheckResults;
}

// The cycle's random source, injected by the server: each request draws the generator its probes use
pub trait RandomSource: Send + Sync {
    fn generator(&self) -> Result<StdRng, SysError>;
}

// The server's source: a generator seeded from the operating system's entropy
pub struct SystemRandom;

impl RandomSource for SystemRandom {
    fn generator(&self) -> Result<StdRng, SysError> {
        StdRng::try_from_rng(&mut SysRng)
    }
}

// The router crate's provider hook, shared by the cycle catalog and the lookup it builds
pub type InferredProvider = dyn Fn(&str) -> Option<String> + Send + Sync;

// The router's catalog, owned so the state stays 'static: the handle is read into a snapshot for each cycle
#[derive(Clone)]
pub struct CycleCatalog {
    pub handle: Arc<CatalogHandle>,
    pub aliases: Arc<AliasMap>,
    pub routing_groups: Arc<RoutingGroupIndex>,
    pub inferred_provider: Arc<InferredProvider>,
}

// B4443's cycle dependencies, injected as the server holds them
#[derive(Clone)]
pub struct HealthCycleState {
    pub provider: Arc<dyn ProviderHealth>,
    pub models: Arc<ModelCatalog>,
    pub policy: ProbePolicy,
    pub catalog: Option<CycleCatalog>,
    pub random: Arc<dyn RandomSource>,
}

// What LiteLLM's health endpoints read from proxy_server globals, handed in by the server that owns them
#[derive(Clone)]
pub struct HealthStatusState {
    pub deployments: Option<Arc<Vec<DeploymentSpec>>>,
    pub router: Option<Arc<dyn HealthRouter>>,
    pub user_model: Option<String>,
    pub health_check_details: DetailVisibility,
    pub health_check_concurrency: Option<usize>,
    pub use_background_health_checks: HealthCheckSource,
    pub skip_disabled_models: SkipDisabledModels,
    pub results: Arc<dyn HealthResultsSource>,
    pub rows: Option<Arc<HealthCheckRows>>,
    pub clock: Arc<dyn UtcClock + Send + Sync>,
    pub tasks: TaskTracker,
    pub cycle: HealthCycleState,
}

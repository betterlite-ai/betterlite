use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_core::api::anthropic_beta_config::BetaConfigLoader;
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_db::gateway_request_tracking::GatewayRequestClock;
use betterlite_models::catalog::CatalogError;
use betterlite_models::fetch_policy::client::CatalogClient;
use betterlite_models::fetch_policy::retry::CatalogSleeper;
use betterlite_models::fetch_policy::retry::JitterSource;
use betterlite_models::reload::CatalogService;
use betterlite_models::reload::FetchLimits;
use betterlite_models::reload::ReloadOutcome;

pub type CostMapFuture<'call> = Pin<Box<dyn Future<Output = ReloadOutcome> + Send + 'call>>;
pub type BetaMatrixFuture<'call> = Pin<Box<dyn Future<Output = Arc<BetaMatrix>> + Send + 'call>>;

// what the checks reach outside this pod: the clock, the cost map fetch (B194) and the beta load (B959)
#[derive(Clone)]
pub struct ReloadSources {
    pub clock: Arc<dyn GatewayRequestClock>,
    pub cost_map: Arc<dyn ModelCostMapSource>,
    pub beta_headers: Arc<dyn BetaHeadersSource>,
}

pub trait ModelCostMapSource: Send + Sync {
    fn refetch_cost_map<'call>(&'call self) -> CostMapFuture<'call>;
}

pub trait BetaHeadersSource: Send + Sync {
    fn reload<'call>(&'call self) -> BetaMatrixFuture<'call>;
}

impl BetaHeadersSource for BetaConfigLoader {
    fn reload<'call>(&'call self) -> BetaMatrixFuture<'call> {
        Box::pin(BetaConfigLoader::reload(self))
    }
}

pub trait JitterFactory: Send + Sync {
    type Jitter: JitterSource;

    fn create(&self) -> Self::Jitter;
}

pub struct CatalogServiceSource<F, C, S, Factory> {
    pub service: Arc<CatalogService<F>>,
    pub limits: FetchLimits,
    pub client: Arc<C>,
    pub sleeper: Arc<S>,
    pub jitter: Factory,
}

impl<F, C, S, Factory> ModelCostMapSource for CatalogServiceSource<F, C, S, Factory>
where
    F: Fn() -> Result<Vec<u8>, CatalogError> + Send + Sync,
    C: CatalogClient,
    S: CatalogSleeper,
    Factory: JitterFactory,
{
    fn refetch_cost_map<'call>(&'call self) -> CostMapFuture<'call> {
        Box::pin(async move {
            // B194 holds the jitter for &mut through every attempt of one fetch, so each poll builds its own
            let mut jitter = self.jitter.create();
            self.service
                .reload(
                    self.limits,
                    self.client.as_ref(),
                    self.sleeper.as_ref(),
                    &mut jitter,
                )
                .await
        })
    }
}

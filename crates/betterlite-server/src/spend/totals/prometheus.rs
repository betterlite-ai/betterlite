use async_trait::async_trait;
use betterlite_logging::prometheus_query::PrometheusQuery;
use betterlite_logging::prometheus_query::clock::Clock;
use betterlite_logging::prometheus_query::daily_spend::{
    PrometheusDailySpend, daily_spend_from_prometheus,
};
use betterlite_logging::prometheus_query::transport::Transport;

#[derive(Debug, thiserror::Error)]
pub enum DailySpendFailure {
    #[error("the Prometheus daily spend query failed")]
    Query,
}

#[async_trait]
pub trait DailySpendSource: Send + Sync {
    async fn daily_spend(
        &self,
        api_key: Option<&str>,
    ) -> Result<Vec<PrometheusDailySpend>, DailySpendFailure>;
}

pub struct PrometheusDailySpendSource<Fetch, Time> {
    pub query: PrometheusQuery<Fetch, Time>,
}

#[async_trait]
impl<Fetch, Time> DailySpendSource for PrometheusDailySpendSource<Fetch, Time>
where
    Fetch: Transport,
    Time: Clock,
{
    async fn daily_spend(
        &self,
        api_key: Option<&str>,
    ) -> Result<Vec<PrometheusDailySpend>, DailySpendFailure> {
        daily_spend_from_prometheus(&self.query, api_key)
            .await
            .map_err(|error| {
                tracing::warn!(error = %error, "the Prometheus daily spend query failed");
                DailySpendFailure::Query
            })
    }
}

use async_trait::async_trait;
use betterlite_logging::prometheus_query::PrometheusQuery;
use betterlite_logging::prometheus_query::clock::Clock;
use betterlite_logging::prometheus_query::daily_spend::{
    PrometheusDailySpend, daily_spend_from_prometheus,
};
use betterlite_logging::prometheus_query::error::QueryError;
use betterlite_logging::prometheus_query::transport::Transport;

#[derive(Debug, thiserror::Error)]
pub enum DailySpendFailure<Failure> {
    #[error("the Prometheus daily spend query failed")]
    Query {
        #[source]
        source: QueryError<Failure>,
    },
}

#[async_trait]
pub trait DailySpendSource: Send + Sync {
    type Failure: std::error::Error + Send + Sync + 'static;

    async fn daily_spend(
        &self,
        api_key: Option<&str>,
    ) -> Result<Vec<PrometheusDailySpend>, Self::Failure>;
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
    type Failure = DailySpendFailure<Fetch::Error>;

    async fn daily_spend(
        &self,
        api_key: Option<&str>,
    ) -> Result<Vec<PrometheusDailySpend>, Self::Failure> {
        daily_spend_from_prometheus(&self.query, api_key)
            .await
            .map_err(|source| DailySpendFailure::Query { source })
    }
}

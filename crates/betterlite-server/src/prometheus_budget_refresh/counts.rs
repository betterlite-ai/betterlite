use betterlite_prometheus::collector_factory::collector::{Collector, MetricValue};
use rust_decimal::Decimal;

use super::source::{BudgetPageError, BudgetPageSource};

mod tests;

pub struct CountGauges<'gauges, Gauge> {
    pub total_users: &'gauges Gauge,
    pub active_users: &'gauges Gauge,
    pub teams: &'gauges Gauge,
}

// LiteLLM's _initialize_user_and_team_count_metrics: total users, then billable users, then teams, and the first failure ends them all
pub async fn refresh_counts<Gauge>(
    source: Option<&dyn BudgetPageSource>,
    gauges: &CountGauges<'_, Gauge>,
) where
    Gauge: Collector + Send + Sync,
{
    let Some(pages) = source else {
        tracing::debug!(
            "Prometheus: skipping user/team count metrics initialization, DB not initialized"
        );
        return;
    };
    if let Err(error) = set_count_gauges(pages, gauges).await {
        tracing::error!("Error initializing user/team count metrics: {error}");
    }
}

async fn set_count_gauges<Gauge>(
    source: &dyn BudgetPageSource,
    gauges: &CountGauges<'_, Gauge>,
) -> Result<(), BudgetPageError>
where
    Gauge: Collector + Send + Sync,
{
    let total_users = source.user_count().await?;
    gauges
        .total_users
        .set(MetricValue::Finite(Decimal::from(total_users)));
    tracing::debug!("Prometheus: set litellm_total_users to {total_users}");
    let billable_users = source.billable_user_count().await?;
    gauges
        .active_users
        .set(MetricValue::Finite(Decimal::from(billable_users)));
    tracing::debug!("Prometheus: set litellm_active_users to {billable_users}");
    let teams = source.team_count().await?;
    gauges.teams.set(MetricValue::Finite(Decimal::from(teams)));
    tracing::debug!("Prometheus: set litellm_teams_count to {teams}");
    Ok(())
}

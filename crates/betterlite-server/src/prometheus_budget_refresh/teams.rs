use async_trait::async_trait;
use betterlite_logging::prometheus::budget_gauges::gauge::EntityBudgetGauges;
use betterlite_logging::prometheus::budget_gauges::inputs::TeamBudgetGaugeInput;
use betterlite_logging::prometheus::budget_gauges::lists::team_list::set_team_list_budget_metrics;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_prometheus::collector_factory::collector::Collector;
use chrono::{DateTime, Utc};

use super::page_walk::{EntityPageWalk, initialize_entity};
use super::source::{BudgetPageError, BudgetPageSource, Page};

mod tests;

struct TeamWalk<'walk, Gauge>
where
    Gauge: Collector + RemovableMetric,
{
    source: &'walk dyn BudgetPageSource,
    gauges: &'walk EntityBudgetGauges<Gauge>,
    labels: LabelSource<'walk>,
    now: DateTime<Utc>,
}

#[async_trait]
impl<Gauge> EntityPageWalk for TeamWalk<'_, Gauge>
where
    Gauge: Collector + RemovableMetric + Send + Sync,
    Gauge::Error: Send + Sync,
{
    type Row = TeamBudgetGaugeInput;

    async fn fetch(&self, page_size: u64, page: u64) -> Result<Page<Self::Row>, BudgetPageError> {
        self.source.teams_page(page_size, page).await
    }

    fn apply(&self, rows: Vec<Self::Row>) -> Result<(), BudgetPageError> {
        set_team_list_budget_metrics(self.gauges, &rows, self.labels, self.now).map_err(|error| {
            BudgetPageError::Gauge {
                source: Box::new(error),
            }
        })
    }
}

pub async fn refresh_teams<Gauge>(
    source: Option<&dyn BudgetPageSource>,
    gauges: &EntityBudgetGauges<Gauge>,
    labels: LabelSource<'_>,
    now: DateTime<Utc>,
) where
    Gauge: Collector + RemovableMetric + Send + Sync,
    Gauge::Error: Send + Sync,
{
    let Some(pages) = source else {
        tracing::debug!("Prometheus: skipping team metrics initialization, DB not initialized");
        return;
    };
    initialize_entity(
        &TeamWalk {
            source: pages,
            gauges,
            labels,
            now,
        },
        "teams",
    )
    .await;
}

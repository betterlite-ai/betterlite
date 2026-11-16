use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

use betterlite_analytics::canonical_date::{PublicError, PublicErrorDetail};
use betterlite_analytics::daily_metrics::{DailySpendData, DailySpendPage, SpendMetrics};
use betterlite_db::daily_activity_contracts::{
    AggregatedRows, DailyActivityProxyReads, DailyActivityScope, KeyMetadataRow, SpendLogsWindow,
};
use betterlite_db::daily_activity_repository::database::DailyActivityDatabase;
use betterlite_db::daily_activity_repository::{
    DailyActivityRepository, DailyActivityRepositoryError,
};
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use tokio::task::JoinError;

mod dispatch;
mod fold;
mod levels;
mod metadata;
mod response;

#[path = "rollups/tests/aggregated_response.rs"]
mod aggregated_response;
#[path = "rollups/tests/async_wrapper.rs"]
mod async_wrapper;
#[path = "rollups/tests/date_ordering.rs"]
mod date_ordering;
#[path = "rollups/tests/dispatch_by_level.rs"]
mod dispatch_by_level;
#[path = "rollups/tests/entity_key_breakdown.rs"]
mod entity_key_breakdown;
#[path = "rollups/tests/entity_rollups.rs"]
mod entity_rollups;
#[path = "rollups/tests/grouping_constants.rs"]
mod grouping_constants;
#[path = "rollups/tests/helper_buckets.rs"]
mod helper_buckets;
#[path = "rollups/tests/provider_flatcost.rs"]
mod provider_flatcost;
#[path = "rollups/tests/ptu_sentinel.rs"]
mod ptu_sentinel;
#[path = "rollups/tests/support.rs"]
mod support;

pub type EntityMetadata = BTreeMap<String, IndexMap<String, Json>>;

struct AggregatedSpendData {
    results: Vec<DailySpendData>,
    totals: SpendMetrics,
}

#[derive(Debug, thiserror::Error)]
enum DailyActivityAggregateError {
    #[error(transparent)]
    Repository(#[from] DailyActivityRepositoryError),
    #[error("rollup row has no date")]
    MissingDate,
    #[error("rollup row date {value} is not a YYYY-MM-DD date")]
    Date { value: String },
    #[error("daily activity aggregation worker failed")]
    Worker {
        #[from]
        source: JoinError,
    },
    #[error("daily activity aggregation worker returned no result")]
    WorkerMissing,
}

pub trait AggregatedActivityRepository: Send + Sync {
    fn read_aggregated(
        &self,
        scope: &DailyActivityScope,
        include_entity_breakdown: bool,
        api_key_limit: i64,
    ) -> impl Future<Output = Result<AggregatedRows, DailyActivityRepositoryError>> + Send;

    fn read_key_metadata(
        &self,
        api_keys: &BTreeSet<String>,
        window: Option<SpendLogsWindow>,
    ) -> impl Future<Output = Result<BTreeMap<String, KeyMetadataRow>, DailyActivityRepositoryError>>
    + Send;
}

impl<Database, Reads, Clock> AggregatedActivityRepository
    for DailyActivityRepository<Database, Reads, Clock>
where
    Database: DailyActivityDatabase,
    Reads: DailyActivityProxyReads + Send + Sync,
    Clock: Fn() -> DateTime<Utc> + Send + Sync,
{
    async fn read_aggregated(
        &self,
        scope: &DailyActivityScope,
        include_entity_breakdown: bool,
        api_key_limit: i64,
    ) -> Result<AggregatedRows, DailyActivityRepositoryError> {
        DailyActivityRepository::aggregated(self, scope, include_entity_breakdown, api_key_limit)
            .await
    }

    async fn read_key_metadata(
        &self,
        api_keys: &BTreeSet<String>,
        window: Option<SpendLogsWindow>,
    ) -> Result<BTreeMap<String, KeyMetadataRow>, DailyActivityRepositoryError> {
        DailyActivityRepository::key_metadata(self, api_keys, window).await
    }
}

pub async fn get_daily_activity_aggregated<Repository>(
    repository: &Repository,
    scope: &DailyActivityScope,
    entity_metadata_field: Option<&EntityMetadata>,
    include_entity_breakdown: bool,
    api_key_limit: i64,
) -> Result<DailySpendPage, PublicError>
where
    Repository: AggregatedActivityRepository,
{
    match aggregate_page(
        repository,
        scope,
        entity_metadata_field,
        include_entity_breakdown,
        api_key_limit,
    )
    .await
    {
        Ok(page) => Ok(page),
        Err(source) => Err(public_error(&source)),
    }
}

async fn aggregate_page<Repository>(
    repository: &Repository,
    scope: &DailyActivityScope,
    entity_metadata_field: Option<&EntityMetadata>,
    include_entity_breakdown: bool,
    api_key_limit: i64,
) -> Result<DailySpendPage, DailyActivityAggregateError>
where
    Repository: AggregatedActivityRepository,
{
    let aggregated_rows = repository
        .read_aggregated(scope, include_entity_breakdown, api_key_limit)
        .await?;
    let entity_rows = aggregated_rows.entity_rows;
    let entity_total_api_keys =
        response::entity_key_totals(entity_rows.as_deref(), include_entity_breakdown);
    let mut aggregated =
        dispatch::aggregate_grouping_rows(repository, aggregated_rows.grouping_rows).await?;
    if let Some(present_rows) = entity_rows.as_deref().filter(|rows| !rows.is_empty()) {
        fold::fold_entity_rollups(
            repository,
            &mut aggregated.results,
            present_rows,
            entity_metadata_field,
        )
        .await?;
    }
    let metadata = response::response_metadata(
        &aggregated.totals,
        api_key_limit,
        aggregated_rows.distinct_api_keys,
        entity_total_api_keys,
    );
    Ok(DailySpendPage {
        results: aggregated.results,
        metadata,
    })
}

fn public_error(source: &DailyActivityAggregateError) -> PublicError {
    tracing::error!("Error fetching aggregated daily activity: {source}");
    PublicError {
        status_code: 500,
        detail: PublicErrorDetail {
            error: format!("Failed to fetch analytics: {source}"),
        },
    }
}

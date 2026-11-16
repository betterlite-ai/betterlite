use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::sync::Arc;

use betterlite_db::shadow_tables::rows::{JobAttemptStats, ShadowJobRow, ShadowTables};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use thiserror::Error;

use super::cache::{ActiveJobs, Clock, JobsCache, JobsCacheError};
use super::jobs::ActiveShadowEvalJob;

#[derive(Clone, Debug, Default)]
pub struct LoadedJobs {
    pub jobs: ActiveJobs,
    pub refilled: bool,
}

#[derive(Debug, Error)]
pub enum JobReadError<DatabaseError>
where
    DatabaseError: StdError + Send + Sync + 'static,
{
    #[error("{source}")]
    Database {
        #[source]
        source: DatabaseError,
    },
    #[error(transparent)]
    Cache(#[from] JobsCacheError),
}

// the sampling path's database and spend counter, injected so a test holds no proxy global
pub struct ShadowJobLoader<Database, Counter>
where
    Database: ShadowTables,
{
    pub cache: JobsCache,
    pub database: Option<Arc<Database>>,
    pub spend: Counter,
}

impl<Database, Counter> ShadowJobLoader<Database, Counter>
where
    Database: ShadowTables,
    Counter: Send + Sync,
{
    pub fn new(
        database: Option<Arc<Database>>,
        spend: Counter,
        clock: Clock,
    ) -> Result<Self, JobsCacheError> {
        let cache = JobsCache::new(clock)?;
        let loader = Self {
            cache,
            database,
            spend,
        };
        Ok(loader)
    }

    #[must_use]
    pub async fn active_jobs(&self, now: DateTime<Utc>) -> LoadedJobs {
        match self.cache.get() {
            Ok(Some(jobs)) => {
                return LoadedJobs {
                    jobs,
                    refilled: false,
                };
            }
            Ok(None) => {}
            Err(error) => {
                tracing::debug!("shadow_eval: active-job read failed: {error}");
                return LoadedJobs::default();
            }
        }
        let Some(database) = self.database.as_deref() else {
            return LoadedJobs::default();
        };
        match read_active_jobs(database, &self.cache, now).await {
            Ok(loaded) => loaded,
            Err(error) => {
                tracing::debug!("shadow_eval: active-job read failed: {error}");
                LoadedJobs::default()
            }
        }
    }
}

async fn read_active_jobs<Database>(
    database: &Database,
    cache: &JobsCache,
    now: DateTime<Utc>,
) -> Result<LoadedJobs, JobReadError<Database::Error>>
where
    Database: ShadowTables,
{
    let rows = database
        .find_active_jobs(now)
        .await
        .map_err(|source| JobReadError::Database { source })?;
    let stats = attempt_stats(database, &rows).await?;
    let jobs = group_jobs(&rows, &stats);
    cache.set(&jobs)?;
    Ok(LoadedJobs {
        jobs,
        refilled: true,
    })
}

async fn attempt_stats<Database>(
    database: &Database,
    rows: &[ShadowJobRow],
) -> Result<Vec<JobAttemptStats>, JobReadError<Database::Error>>
where
    Database: ShadowTables,
{
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
    database
        .attempt_stats(&ids)
        .await
        .map_err(|source| JobReadError::Database { source })
}

fn group_jobs(rows: &[ShadowJobRow], stats: &[JobAttemptStats]) -> ActiveJobs {
    let mut by_job: BTreeMap<&str, (i64, Decimal)> = BTreeMap::new();
    for stat in stats {
        by_job.insert(stat.job_id.as_str(), (stat.attempts, stat.spend));
    }
    let mut jobs: ActiveJobs = BTreeMap::new();
    for row in rows {
        let (attempts, spend) = by_job
            .get(row.id.as_str())
            .copied()
            .unwrap_or((0, Decimal::ZERO));
        match ActiveShadowEvalJob::from_row(row, attempts, spend) {
            Ok(job) => jobs
                .entry((row.target_type.clone(), row.target_id.clone()))
                .or_default()
                .push(job),
            Err(error) => tracing::debug!("shadow_eval: skipping unsamplable job row: {error}"),
        }
    }
    jobs
}

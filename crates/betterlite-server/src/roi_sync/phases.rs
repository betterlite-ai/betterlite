use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_db::search_tools::convert::isoformat;
use betterlite_roi::estimator::cache::cache_context;
use betterlite_roi::estimator::json::JsonError;
use betterlite_roi::estimator::models::EstimatorModel;
use betterlite_roi::records::{ROIPullRecord, ROIReport};
use betterlite_roi::settings::ROISettings;
use chrono::TimeDelta;
use futures_util::future::join_all;

use crate::roi_cache::key::cache_key;
use crate::roi_cache::processed::processed_records;
use crate::roi_cache::repositories::{RepositoryBatch, read_repositories};
use crate::roi_cache::store::{ReportRepository, previous_report};
use crate::roi_estimator::{CompletionCaller, Estimator};
use crate::roi_github::client::GitHub;
use crate::roi_github::transport::Transport;
use crate::roi_spend::SpendReader;

use super::PR_CONCURRENCY;
use super::capabilities::{CapabilityLookup, SessionCapabilities};
use super::error::RunError;
use super::ports::SyncClock;
use super::process::{EstimatePlan, IndexedPull, PullProcessor};
use super::report::{ReportParts, build_report};
use super::status::{StatusChange, StatusReporter};

pub(crate) struct RunContext<'run, T: Transport> {
    pub(crate) settings: &'run ROISettings,
    pub(crate) models: Option<&'run [EstimatorModel]>,
    pub(crate) complete: &'run CompletionCaller,
    pub(crate) github: &'run GitHub<T>,
    pub(crate) status: &'run StatusReporter,
    pub(crate) clock: &'run SyncClock,
    pub(crate) capabilities: &'run Arc<dyn CapabilityLookup>,
}

pub(crate) struct Execution {
    pub(crate) report: ROIReport,
    pub(crate) stage: &'static str,
}

pub(crate) async fn execute<Reader, T, Repository>(
    context: &RunContext<'_, T>,
    repository: &Repository,
    spend_reader: &Reader,
) -> Result<Execution, RunError<Repository::Error>>
where
    T: Transport,
    Reader: SpendReader + Sync,
    Repository: ReportRepository + Sync,
{
    let end = (context.clock)().date_naive();
    let shift = TimeDelta::try_days(context.settings.backfill_days.saturating_sub(1))
        .ok_or(RunError::DateRange)?;
    let start = end.checked_sub_signed(shift).ok_or(RunError::DateRange)?;
    let spend = spend_reader
        .read(start, end)
        .await
        .map_err(RunError::Spend)?;
    context.status.apply(StatusChange::Repositories);
    let repositories =
        read_repositories(context.github, &context.settings.repos, start, end).await?;
    let previous = previous_report(repository)
        .await
        .map_err(RunError::Repository)?;
    let plan = index_pulls(context, previous, &repositories)
        .await
        .map_err(RunError::Key)?;
    let total = i64::try_from(plan.queue.len()).map_err(RunError::Count)?;
    context.status.apply(StatusChange::Estimates { total });
    let processed = process_pulls(context, repository, &plan).await?;
    let report = build_report(ReportParts {
        settings: context.settings,
        start,
        end,
        synced_at: isoformat((context.clock)()),
        spend,
        processed,
        pull_count: plan.queue.len(),
        repositories: &repositories,
    })?;
    Ok(Execution {
        report,
        stage: repositories.stage,
    })
}

async fn index_pulls<T: Transport>(
    context: &RunContext<'_, T>,
    previous: Option<ROIReport>,
    repositories: &RepositoryBatch,
) -> Result<EstimatePlan, JsonError> {
    let mut predicates = SessionCapabilities {
        lookup: context.capabilities.as_ref(),
    };
    let estimate_context = cache_context(context.settings, context.models, &mut predicates).await?;
    let previous_pulls = previous.map_or_else(BTreeMap::new, |report| {
        let mut map = BTreeMap::new();
        for pull in report.pulls {
            if let Some(key) = pull.cache_key.clone() {
                map.insert(key, pull);
            }
        }
        map
    });
    let mut indexed = Vec::with_capacity(repositories.queue.len());
    for (position, (repo, pull)) in repositories.queue.iter().enumerate() {
        let key = cache_key(context.settings, &estimate_context, repo, pull)?;
        indexed.push(IndexedPull {
            position,
            repo: repo.clone(),
            pull: pull.clone(),
            key,
        });
    }
    Ok(EstimatePlan {
        queue: indexed,
        estimate_context,
        previous: previous_pulls,
    })
}

async fn process_pulls<T, Repository>(
    context: &RunContext<'_, T>,
    repository: &Repository,
    plan: &EstimatePlan,
) -> Result<BTreeMap<usize, ROIPullRecord>, RunError<Repository::Error>>
where
    T: Transport,
    Repository: ReportRepository + Sync,
{
    let estimator = Estimator::new(
        context.settings.clone(),
        context.complete.clone(),
        context.models,
    );
    let processor = PullProcessor {
        github: context.github,
        repository,
        estimator: &estimator,
        settings: context.settings,
        estimate_context: &plan.estimate_context,
        previous: &plan.previous,
        status: context.status,
        capabilities: context.capabilities,
    };
    let workers =
        join_all((0..PR_CONCURRENCY).map(|offset| processor.worker(offset, &plan.queue))).await;
    let mut processed = Vec::new();
    for worker in workers {
        processed.extend(worker?);
    }
    processed_records(processed).map_err(RunError::from)
}

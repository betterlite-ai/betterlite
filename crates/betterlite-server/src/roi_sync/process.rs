use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_roi::records::{ROIEstimateStatus, ROIPullEvidence, ROIPullRecord};
use betterlite_roi::settings::ROISettings;

use crate::roi_cache::fallback::{estimate_with_fallback, unavailable_record};
use crate::roi_cache::key::cache_key;
use crate::roi_cache::processed::ProcessedPull;
use crate::roi_cache::records::{cached_record, report_record};
use crate::roi_cache::store::{PULL_PARAM_PREFIX, ReportRepository, cache_estimated_pull};
use crate::roi_estimator::Estimator;
use crate::roi_evidence::evidence;
use crate::roi_github::client::GitHub;
use crate::roi_github::models::{GitHubHead, GitHubPullListItem, GitHubUser};
use crate::roi_github::profile_email::profile_email;
use crate::roi_github::transport::Transport;

use super::PR_CONCURRENCY;
use super::capabilities::{CapabilityLookup, SessionCapabilities};
use super::error::RunError;
use super::status::StatusReporter;

pub(crate) struct IndexedPull {
    pub(crate) position: usize,
    pub(crate) repo: String,
    pub(crate) pull: GitHubPullListItem,
    pub(crate) key: Option<String>,
}

pub(crate) struct EstimatePlan {
    pub(crate) queue: Vec<IndexedPull>,
    pub(crate) estimate_context: String,
    pub(crate) previous: BTreeMap<String, ROIPullRecord>,
}

pub(crate) struct PullProcessor<'run, T: Transport, Repository> {
    pub(crate) github: &'run GitHub<T>,
    pub(crate) repository: &'run Repository,
    pub(crate) estimator: &'run Estimator,
    pub(crate) settings: &'run ROISettings,
    pub(crate) estimate_context: &'run str,
    pub(crate) previous: &'run BTreeMap<String, ROIPullRecord>,
    pub(crate) status: &'run StatusReporter,
    pub(crate) capabilities: &'run Arc<dyn CapabilityLookup>,
}

impl<T, Repository> PullProcessor<'_, T, Repository>
where
    T: Transport,
    Repository: ReportRepository + Sync,
{
    pub(crate) async fn worker(
        &self,
        offset: usize,
        queue: &[IndexedPull],
    ) -> Result<Vec<ProcessedPull>, RunError<Repository::Error>> {
        let mut processed = Vec::new();
        for item in queue.iter().skip(offset).step_by(PR_CONCURRENCY) {
            processed.push(self.process(item).await?);
        }
        Ok(processed)
    }

    async fn process(
        &self,
        item: &IndexedPull,
    ) -> Result<ProcessedPull, RunError<Repository::Error>> {
        let saved = self.saved_record(item).await?;
        let cached = match saved.as_ref() {
            Some(value) => {
                let bytes = serde_json::to_vec(value)?;
                Some(serde_json::from_slice::<ROIPullRecord>(&bytes)?)
            }
            None => item
                .key
                .as_deref()
                .and_then(|key| self.previous.get(key))
                .cloned(),
        };
        let reusable = cached.as_ref().filter(|pull| {
            pull.estimate.status == ROIEstimateStatus::Estimated && pull.commit_emails.is_some()
        });
        if let Some(pull) = reusable {
            return self.reuse(item, pull, saved.is_some()).await;
        }
        self.estimate_new(item).await
    }

    async fn saved_record(
        &self,
        item: &IndexedPull,
    ) -> Result<Option<JsonData>, RunError<Repository::Error>> {
        let Some(key) = item.key.as_deref() else {
            return Ok(None);
        };
        let name = format!("{PULL_PARAM_PREFIX}{key}");
        self.repository
            .get_param(&name)
            .await
            .map_err(RunError::Repository)
    }

    async fn reuse(
        &self,
        item: &IndexedPull,
        cached: &ROIPullRecord,
        saved: bool,
    ) -> Result<ProcessedPull, RunError<Repository::Error>> {
        let profile = profile_email(self.github, &cached.login, &cached.profile_email).await;
        let record = refreshed_record(cached, &profile);
        let previous = if saved { Some(cached) } else { None };
        cache_estimated_pull(self.repository, item.key.as_deref(), &record, previous).await?;
        self.status.record_estimate(&record.estimate);
        Ok(ProcessedPull {
            position: item.position,
            record,
            metadata_unavailable: false,
        })
    }

    async fn estimate_new(
        &self,
        item: &IndexedPull,
    ) -> Result<ProcessedPull, RunError<Repository::Error>> {
        let pull_evidence = match evidence(self.github, &item.repo, &item.pull).await {
            Ok(found) => found,
            Err(error) => {
                let record = unavailable_record(self.github, &item.repo, &item.pull, &error).await;
                self.status.record_estimate(&record.estimate);
                return Ok(ProcessedPull {
                    position: item.position,
                    record,
                    metadata_unavailable: true,
                });
            }
        };
        let mut predicates = SessionCapabilities {
            lookup: self.capabilities.as_ref(),
        };
        let estimate =
            estimate_with_fallback(self.estimator, &pull_evidence, &mut predicates).await;
        let evidence_item = evidence_item(&pull_evidence);
        let fetched_key = cache_key(
            self.settings,
            self.estimate_context,
            &item.repo,
            &evidence_item,
        )
        .map_err(RunError::Key)?;
        let record = report_record(&pull_evidence, estimate, fetched_key);
        cache_estimated_pull(self.repository, record.cache_key.as_deref(), &record, None).await?;
        self.status.record_estimate(&record.estimate);
        Ok(ProcessedPull {
            position: item.position,
            record,
            metadata_unavailable: false,
        })
    }
}

fn refreshed_record(cached: &ROIPullRecord, profile: &str) -> ROIPullRecord {
    let mut record = cached_record(cached);
    record.profile_email = profile.to_owned();
    let mut emails: BTreeSet<String> = cached
        .commit_emails
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|email| !email.is_empty())
        .collect();
    if !profile.is_empty() {
        emails.insert(profile.to_owned());
    }
    record.emails = emails.into_iter().collect();
    record
}

fn evidence_item(pull: &ROIPullEvidence) -> GitHubPullListItem {
    GitHubPullListItem {
        number: pull.number,
        html_url: String::new(),
        merged_at: Some(pull.merged_at.clone()),
        updated_at: pull.merged_at.clone(),
        title: pull.title.clone(),
        body: Some(pull.body.clone()),
        head: Some(GitHubHead {
            sha: pull.head_sha.clone(),
        }),
        user: Some(GitHubUser {
            login: Some(pull.login.clone()),
        }),
    }
}

use betterlite_models::model_capability_predicates::CapabilityPredicates;
use betterlite_roi::records::{ROIEstimate, ROIEstimateStatus, ROIPullEvidence, ROIPullRecord};

use crate::roi_estimator::Estimator;
use crate::roi_github::client::GitHub;
use crate::roi_github::error::SourceError;
use crate::roi_github::models::GitHubPullListItem;
use crate::roi_github::profile_email::profile_email;
use crate::roi_github::transport::Transport;

const DELETED_USER: &str = "deleted-user";

fn sourceless_estimate(status: ROIEstimateStatus, reasoning: String) -> ROIEstimate {
    ROIEstimate {
        status,
        hours: None,
        reasoning,
        model: None,
        evidence_source: None,
        effort_basis: None,
        cached: None,
    }
}

fn login_or_deleted(pull: &GitHubPullListItem) -> &str {
    pull.user
        .as_ref()
        .and_then(|user| user.login.as_deref())
        .filter(|login| !login.is_empty())
        .unwrap_or(DELETED_USER)
}

pub async fn estimate_with_fallback<Predicates>(
    estimator: &Estimator,
    evidence: &ROIPullEvidence,
    predicates: &mut Predicates,
) -> ROIEstimate
where
    Predicates: CapabilityPredicates + Send,
{
    match estimator.estimate(evidence, predicates).await {
        Ok(estimate) => estimate,
        Err(error) => sourceless_estimate(ROIEstimateStatus::Error, error.to_string()),
    }
}

pub async fn unavailable_record<T>(
    github: &GitHub<T>,
    repo: &str,
    pull: &GitHubPullListItem,
    error: &SourceError,
) -> ROIPullRecord
where
    T: Transport,
{
    let login = login_or_deleted(pull);
    let profile = profile_email(github, login, "").await;
    let emails = if profile.is_empty() {
        Vec::new()
    } else {
        vec![profile.clone()]
    };
    let reasoning =
        format!("PR metadata could not be read: {error} Run analysis again to retry this PR.");
    ROIPullRecord {
        repo: repo.to_owned(),
        number: pull.number,
        title: pull.title.clone(),
        url: pull.html_url.clone(),
        login: login.to_owned(),
        emails,
        profile_email: profile,
        commit_emails: Some(Vec::new()),
        merged_at: pull
            .merged_at
            .clone()
            .unwrap_or_else(|| pull.updated_at.clone()),
        head_sha: pull
            .head
            .as_ref()
            .map_or_else(String::new, |head| head.sha.clone()),
        additions: 0,
        deletions: 0,
        changed_files: 0,
        commit_count: 0,
        incomplete_metadata: true,
        estimate: sourceless_estimate(ROIEstimateStatus::NeedsReview, reasoning),
        cache_key: None,
    }
}

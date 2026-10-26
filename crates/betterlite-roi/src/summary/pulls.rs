use std::collections::BTreeSet;

use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::records::{ROIEstimate, ROIPullRecord, ROIReport};
use crate::settings::normalize_email;
use crate::summary::error::SummaryError;
use crate::summary::identity::{PullIdentity, match_identity};
use crate::summary::models::{ROIEstimateResponse, ROIPullSummary};

fn estimate_response(estimate: &ROIEstimate) -> Result<ROIEstimateResponse, SummaryError> {
    let hours = estimate
        .hours
        .map(|value| {
            Decimal::try_from(value).map_err(|source| SummaryError::Hours { value, source })
        })
        .transpose()?;
    Ok(ROIEstimateResponse {
        status: estimate.status,
        hours,
        reasoning: estimate.reasoning.clone(),
        model: estimate.model.clone(),
        evidence_source: estimate.evidence_source.clone(),
        effort_basis: estimate.effort_basis.clone(),
        cached: estimate.cached.unwrap_or(false),
    })
}

pub(super) fn pull_summaries(
    report: &ROIReport,
    mappings: &IndexMap<String, String>,
) -> Result<Vec<ROIPullSummary>, SummaryError> {
    let observed_emails: BTreeSet<String> = report
        .spend
        .iter()
        .map(|row| normalize_email(&row.email))
        .filter(|email| !email.is_empty())
        .collect();
    report
        .pulls
        .iter()
        .map(|pull| {
            let identity = match_identity(pull, &observed_emails, mappings);
            pull_summary(pull, identity, &observed_emails)
        })
        .collect()
}

fn pull_summary(
    pull: &ROIPullRecord,
    identity: PullIdentity,
    observed_emails: &BTreeSet<String>,
) -> Result<ROIPullSummary, SummaryError> {
    Ok(ROIPullSummary {
        repo: pull.repo.clone(),
        number: pull.number,
        title: pull.title.clone(),
        url: pull.url.clone(),
        login: pull.login.clone(),
        emails: pull.emails.clone(),
        profile_email: pull.profile_email.clone(),
        merged_at: pull.merged_at.clone(),
        head_sha: pull.head_sha.clone(),
        additions: pull.additions,
        deletions: pull.deletions,
        changed_files: pull.changed_files,
        commit_count: pull.commit_count,
        incomplete_metadata: pull.incomplete_metadata,
        estimate: estimate_response(&pull.estimate)?,
        cache_key: pull.cache_key.clone(),
        matched: observed_emails.contains(&identity.email),
        email: identity.email,
        match_method: identity.method,
    })
}

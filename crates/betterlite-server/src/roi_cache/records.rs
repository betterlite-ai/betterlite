use betterlite_roi::records::{ROIEstimate, ROIPullEvidence, ROIPullRecord};

pub fn cached_record(pull: &ROIPullRecord) -> ROIPullRecord {
    ROIPullRecord {
        repo: pull.repo.clone(),
        number: pull.number,
        title: pull.title.clone(),
        url: pull.url.clone(),
        login: pull.login.clone(),
        emails: pull.emails.clone(),
        profile_email: pull.profile_email.clone(),
        commit_emails: Some(pull.commit_emails.clone().unwrap_or_default()),
        merged_at: pull.merged_at.clone(),
        head_sha: pull.head_sha.clone(),
        additions: pull.additions,
        deletions: pull.deletions,
        changed_files: pull.changed_files,
        commit_count: pull.commit_count,
        incomplete_metadata: pull.incomplete_metadata,
        estimate: ROIEstimate {
            status: pull.estimate.status,
            hours: pull.estimate.hours,
            reasoning: pull.estimate.reasoning.clone(),
            model: pull.estimate.model.clone(),
            evidence_source: pull.estimate.evidence_source.clone(),
            effort_basis: pull.estimate.effort_basis.clone(),
            cached: Some(true),
        },
        cache_key: pull.cache_key.clone(),
    }
}

pub fn report_record(
    evidence: &ROIPullEvidence,
    estimate: ROIEstimate,
    key: Option<String>,
) -> ROIPullRecord {
    ROIPullRecord {
        repo: evidence.repo.clone(),
        number: evidence.number,
        title: evidence.title.clone(),
        url: evidence.url.clone(),
        login: evidence.login.clone(),
        emails: evidence.emails.clone(),
        profile_email: evidence.profile_email.clone(),
        commit_emails: Some(evidence.commit_emails.clone().unwrap_or_default()),
        merged_at: evidence.merged_at.clone(),
        head_sha: evidence.head_sha.clone(),
        additions: evidence.additions,
        deletions: evidence.deletions,
        changed_files: evidence.changed_files,
        commit_count: evidence.commit_count,
        incomplete_metadata: evidence.incomplete_metadata,
        estimate,
        cache_key: key,
    }
}

use crate::models::{
    Evidence, Finding, FindingDraft, Job, Lens, LensSettings, Scope, Worker, finding_identity,
    legacy_finding_identity,
};
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

#[must_use]
pub fn can_access(viewer: &Scope, target: &Scope) -> bool {
    viewer.all_teams
        || (!target.all_teams
            && viewer.team_id == target.team_id
            && (!viewer.team_id.is_empty() || viewer.api_key_hash == target.api_key_hash))
}
#[must_use]
pub fn current_job(lens: &Lens) -> Option<&Job> {
    lens.jobs
        .iter()
        .find(|job| matches!(job.status.as_str(), "queued" | "running"))
}
#[must_use]
pub fn queue_job(
    lens: &Lens,
    now: DateTime<Utc>,
    job_id: String,
    lookback_hours: Option<u32>,
    settings: Option<LensSettings>,
) -> Lens {
    if current_job(lens).is_some() {
        return lens.clone();
    }
    let chosen = settings.unwrap_or_else(|| lens.settings.clone());
    let hours = lookback_hours.unwrap_or(chosen.lookback_hours);
    let job = Job {
        id: job_id,
        status: "queued".to_owned(),
        stage: "Queued".to_owned(),
        created_at: now,
        start: now
            .checked_sub_signed(Duration::hours(i64::from(hours)))
            .unwrap_or(now),
        end: now.checked_sub_signed(Duration::minutes(2)).unwrap_or(now),
        settings: chosen,
        revision: lens.revision,
        worker_id: None,
        lease_until: None,
        attempts: 0,
        finished_at: None,
        coverage: crate::models::Coverage::default(),
        error: String::new(),
        sample: None,
        cost: Decimal::ZERO,
        findings: None,
        assessments: Vec::new(),
    };
    let mut updated = lens.clone();
    updated.jobs = vec![job];
    updated
}
#[must_use]
pub fn claim_job(lens: &Lens, worker: &Worker, now: DateTime<Utc>) -> Lens {
    let Some(job) = current_job(lens).cloned() else {
        return lens.clone();
    };
    if !can_access(&worker.scope, &lens.scope)
        || (job.status == "running" && job.lease_until.is_some_and(|until| until > now))
    {
        return lens.clone();
    }
    let mut updated = lens.clone();
    if job.attempts >= 3 {
        let mut failed = job;
        failed.status = "failed".to_owned();
        failed.stage = "Failed".to_owned();
        failed.error = "Worker disconnected repeatedly".to_owned();
        failed.finished_at = Some(now);
        replace_job(&mut updated, failed);
        updated.next_run_at = now
            .checked_add_signed(Duration::minutes(i64::from(lens.settings.interval_minutes)))
            .unwrap_or(now);
        return updated;
    }
    let mut claimed = job;
    claimed.status = "running".to_owned();
    claimed.stage = "Collecting executions".to_owned();
    claimed.worker_id = Some(worker.id.clone());
    claimed.lease_until = Some(now.checked_add_signed(Duration::minutes(5)).unwrap_or(now));
    claimed.attempts = claimed.attempts.saturating_add(1);
    replace_job(&mut updated, claimed);
    updated
}
#[must_use]
pub fn renew_budget(lens: &Lens, now: DateTime<Utc>) -> Lens {
    let month = now.format("%Y-%m").to_string();
    if lens.budget_month == month {
        return lens.clone();
    }
    let mut updated = lens.clone();
    updated.budget_month = month;
    updated.spent = Decimal::ZERO;
    updated
}
#[must_use]
pub fn merge_finding(
    lens: &Lens,
    draft: &FindingDraft,
    revision: u64,
    now: DateTime<Utc>,
) -> Finding {
    let identity = finding_identity(&lens.id, &draft.check_id, &draft.kind, &draft.title);
    let legacy = legacy_finding_identity(&lens.id, &draft.check_id, &draft.title);
    let previous = lens.findings.iter().find(|finding| {
        (draft.existing_finding_id.as_ref() == Some(&finding.id)
            || finding.id == identity
            || finding.id == legacy)
            && finding.kind == draft.kind
            && finding.check_id == draft.check_id
    });
    let occurrences = support_occurrences(draft);
    let Some(previous) = previous else {
        return Finding {
            title: draft.title.clone(),
            description: draft.description.clone(),
            check_id: draft.check_id.clone(),
            kind: draft.kind.clone(),
            priority: draft.priority.clone(),
            suggestion: draft.suggestion.clone(),
            limitation: draft.limitation.clone(),
            evidence: draft.evidence.clone(),
            existing_finding_id: draft.existing_finding_id.clone(),
            id: identity,
            status: "open".to_owned(),
            reason: String::new(),
            first_seen: now,
            last_seen: now,
            occurrences,
            revision,
        };
    };
    let new_occurrence = occurrences
        .iter()
        .any(|item| !previous.occurrences.contains(item));
    let mut merged = previous.clone();
    merged.last_seen = if new_occurrence {
        now
    } else {
        previous.last_seen
    };
    merged.occurrences.extend(occurrences);
    merged.occurrences.sort();
    merged.occurrences.dedup();
    merged.evidence = dedup_evidence(&previous.evidence, &draft.evidence);
    if previous.status == "resolved" && new_occurrence {
        merged.status = "open".to_owned();
    }
    merged
}
#[must_use]
pub fn snapshot_finding(
    lens: &Lens,
    draft: &FindingDraft,
    revision: u64,
    now: DateTime<Utc>,
) -> Finding {
    let mut merged = merge_finding(lens, draft, revision, now);
    merged.title.clone_from(&draft.title);
    merged.description.clone_from(&draft.description);
    merged.check_id.clone_from(&draft.check_id);
    merged.kind.clone_from(&draft.kind);
    merged.priority.clone_from(&draft.priority);
    merged.suggestion.clone_from(&draft.suggestion);
    merged.limitation.clone_from(&draft.limitation);
    merged.evidence.clone_from(&draft.evidence);
    merged
        .existing_finding_id
        .clone_from(&draft.existing_finding_id);
    merged.first_seen = now;
    merged.last_seen = now;
    merged.occurrences = support_occurrences(draft);
    merged.revision = revision;
    merged
}
fn replace_job(lens: &mut Lens, replacement: Job) {
    if let Some(job) = lens.jobs.iter_mut().find(|job| job.id == replacement.id) {
        *job = replacement;
    }
}
fn support_occurrences(draft: &FindingDraft) -> Vec<String> {
    let mut ids: Vec<String> = draft
        .evidence
        .iter()
        .filter(|evidence| evidence.role == "support")
        .map(|evidence| evidence.execution_id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}
fn dedup_evidence(previous: &[Evidence], current: &[Evidence]) -> Vec<Evidence> {
    let mut values = BTreeMap::new();
    for evidence in previous.iter().chain(current) {
        values.insert(
            (
                evidence.execution_id.clone(),
                evidence.span_id.clone(),
                evidence.quote.clone(),
            ),
            evidence.clone(),
        );
    }
    values.into_values().take(20).collect()
}

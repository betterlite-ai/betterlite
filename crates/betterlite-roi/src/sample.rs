use chrono::{DateTime, Days, NaiveDate, SecondsFormat, Utc};
use rust_decimal::Decimal;

use crate::records::{ROIEstimate, ROIEstimateStatus, ROIPullRecord, ROIReport, ROISpendRecord};
use crate::settings::DEFAULT_PROMPT;

const REPO: &str = "example/gateway";
const ESTIMATOR_MODEL: &str = "your-estimator-model";
const ESTIMATE_REASONING: &str = "Sample estimate of engineering effort without AI assistance. Live estimates use PR descriptions, file change counts, and commit metadata.";

struct Example {
    index: i64,
    login: &'static str,
    email: &'static str,
    title: &'static str,
    hours: f64,
    // 18.2 is 182 tenths, so the Decimal is exact where an f64 is not
    cost_in_tenths: i64,
    merge_days_before_end: u64,
}

// start is 29 days before now, so a merge 2, 4 or 6 days after it lands 27, 25 and 23 days before now
const EXAMPLES: [Example; 3] = [
    Example {
        index: 0,
        login: "alex",
        email: "alex@example.com",
        title: "Add usage breakdown by model",
        hours: 6.5,
        cost_in_tenths: 182,
        merge_days_before_end: 27,
    },
    Example {
        index: 1,
        login: "jordan",
        email: "jordan@example.com",
        title: "Fix streaming response cancellation",
        hours: 4.0,
        cost_in_tenths: 128,
        merge_days_before_end: 25,
    },
    Example {
        index: 2,
        login: "casey",
        email: "",
        title: "Add integration tests for billing",
        hours: 5.5,
        cost_in_tenths: 0,
        merge_days_before_end: 23,
    },
];

fn estimate(hours: f64) -> ROIEstimate {
    ROIEstimate {
        status: ROIEstimateStatus::Estimated,
        hours: Some(hours),
        reasoning: ESTIMATE_REASONING.to_owned(),
        model: Some(ESTIMATOR_MODEL.to_owned()),
        effort_basis: Some("without_ai".to_owned()),
        evidence_source: Some("pr_metadata".to_owned()),
        cached: Some(false),
    }
}

fn pull_record(example: &Example, merged: NaiveDate) -> ROIPullRecord {
    ROIPullRecord {
        repo: REPO.to_owned(),
        number: 142_i64.saturating_add(example.index),
        title: example.title.to_owned(),
        url: String::new(),
        login: example.login.to_owned(),
        emails: if example.email.is_empty() {
            Vec::new()
        } else {
            vec![example.email.to_owned()]
        },
        profile_email: example.email.to_owned(),
        commit_emails: None,
        merged_at: format!("{merged}T14:20:00Z"),
        head_sha: format!("sample-{}", example.index),
        additions: 47_i64.saturating_add(example.index.saturating_mul(23)),
        deletions: 12_i64.saturating_add(example.index.saturating_mul(4)),
        changed_files: 3,
        commit_count: 1,
        incomplete_metadata: false,
        estimate: estimate(example.hours),
        cache_key: None,
    }
}

fn spend_record(example: &Example, merged: NaiveDate) -> ROISpendRecord {
    ROISpendRecord {
        date: merged.to_string(),
        user_id: example.login.to_owned(),
        email: example.email.to_owned(),
        spend: Decimal::new(example.cost_in_tenths, 1),
        requests: 150,
    }
}

fn iso_timestamp(now: DateTime<Utc>) -> String {
    let precision = if now.timestamp_subsec_micros() == 0 {
        SecondsFormat::Secs
    } else {
        SecondsFormat::Micros
    };
    now.to_rfc3339_opts(precision, false)
}

#[must_use]
pub fn sample_report(now: DateTime<Utc>) -> ROIReport {
    let end = now.date_naive();
    // Python's date subtraction raises an OverflowError before year 1, so a demo date that far back clamps instead
    let start = end
        .checked_sub_days(Days::new(29))
        .unwrap_or(NaiveDate::MIN);
    let mut pulls = Vec::with_capacity(EXAMPLES.len());
    let mut spend = Vec::new();
    for example in &EXAMPLES {
        let merged = end
            .checked_sub_days(Days::new(example.merge_days_before_end))
            .unwrap_or(NaiveDate::MIN);
        pulls.push(pull_record(example, merged));
        if !example.email.is_empty() {
            spend.push(spend_record(example, merged));
        }
    }
    ROIReport {
        mode: "demo".to_owned(),
        start: start.to_string(),
        end: end.to_string(),
        synced_at: iso_timestamp(now),
        repos: vec![REPO.to_owned()],
        estimator_model: ESTIMATOR_MODEL.to_owned(),
        estimator_prompt: DEFAULT_PROMPT.to_owned(),
        effort_basis: "without_ai".to_owned(),
        spend,
        pulls,
        settings_fingerprint: "sample".to_owned(),
        warnings: None,
        unavailable_repos: None,
        id: None,
    }
}

mod tests;

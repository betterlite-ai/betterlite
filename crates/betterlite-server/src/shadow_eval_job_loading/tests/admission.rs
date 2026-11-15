#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, TimeZone as _, Utc};
use rust_decimal::Decimal;

use super::super::admission::{
    canonical_group, direction_admits, job_spend_counter_key, sample_hits, scope_admits,
};
use super::super::jobs::ShadowEvalDirection;
use super::super::sampling::{RequestAdmission, sampled_jobs};
use super::support::active_job;

fn instant() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap()
}

fn later() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 2, 0, 0, 0).unwrap()
}

fn sample(request_id: &str) -> RequestAdmission<'_> {
    RequestAdmission {
        routed_by: Some("incumbent-router"),
        model_group: "gpt-4o",
        request_id,
    }
}

#[test]
fn shadow_eval_admission_forward_skips_traffic_any_arm_served() {
    let mut job = active_job("job-a", ShadowEvalDirection::Forward, 100.0_f64);
    job.arm_router_names = vec!["router-a".to_owned(), "router-b".to_owned()];
    assert!(!direction_admits(&job, Some("router-a")));
    assert!(!direction_admits(&job, Some("router-b")));
    assert!(direction_admits(&job, Some("router-c")));
    assert!(direction_admits(&job, None));
}

#[test]
fn shadow_eval_admission_reverse_takes_only_its_router() {
    let job = active_job("job-a", ShadowEvalDirection::Reverse, 100.0_f64);
    assert!(direction_admits(&job, Some("router-a")));
    assert!(!direction_admits(&job, Some("router-b")));
    assert!(!direction_admits(&job, None));
}

#[test]
fn shadow_eval_admission_counts_only_a_dice_miss_as_not_sampled() {
    let now = instant();
    let mut expired = active_job("job-expired", ShadowEvalDirection::Forward, 100.0_f64);
    expired.ends_at = now;
    let mut spent = active_job("job-spent", ShadowEvalDirection::Forward, 100.0_f64);
    spent.ends_at = later();
    spent.max_budget = Some(Decimal::ONE);
    spent.spend = Decimal::ONE;
    let mut exhausted = active_job("job-exhausted", ShadowEvalDirection::Forward, 100.0_f64);
    exhausted.ends_at = later();
    exhausted.attempts = 2;
    exhausted.max_turns = 2;
    let mut wrong_direction = active_job("job-wrong-way", ShadowEvalDirection::Reverse, 100.0_f64);
    wrong_direction.ends_at = later();
    let mut out_of_scope = active_job("job-out-of-scope", ShadowEvalDirection::Forward, 100.0_f64);
    out_of_scope.ends_at = later();
    out_of_scope.models = BTreeSet::from(["other-model".to_owned()]);
    let mut miss = active_job("job-miss", ShadowEvalDirection::Forward, 0.0_f64);
    miss.ends_at = later();
    let mut hit = active_job("job-hit", ShadowEvalDirection::Forward, 100.0_f64);
    hit.ends_at = later();
    let jobs = [
        &expired,
        &spent,
        &exhausted,
        &wrong_direction,
        &out_of_scope,
        &miss,
        &hit,
    ];
    let starts = BTreeMap::new();
    let aliases = BTreeMap::new();
    let sampled = sampled_jobs(&jobs, &sample("req-1"), &starts, &aliases, now);
    assert_eq!(
        sampled
            .eligible
            .iter()
            .map(|job| job.id.as_str())
            .collect::<Vec<_>>(),
        vec!["job-hit"]
    );
    assert_eq!(sampled.not_sampled, vec!["job-miss"]);
}

#[test]
fn shadow_eval_admission_turn_gate_adds_the_job_starts() {
    let now = instant();
    let mut job = active_job("job-a", ShadowEvalDirection::Forward, 100.0_f64);
    job.ends_at = later();
    job.attempts = 2;
    job.max_turns = 3;
    let jobs = [&job];
    let aliases = BTreeMap::new();
    let mut starts = BTreeMap::new();
    let without = sampled_jobs(&jobs, &sample("req-1"), &starts, &aliases, now);
    starts.insert("job-a".to_owned(), 1);
    let with = sampled_jobs(&jobs, &sample("req-1"), &starts, &aliases, now);
    assert_eq!(without.eligible.len(), 1);
    assert!(with.eligible.is_empty());
    assert!(with.not_sampled.is_empty());
}

#[test]
fn shadow_eval_admission_budget_gate_closes_at_the_limit() {
    let now = instant();
    let mut job = active_job("job-a", ShadowEvalDirection::Forward, 100.0_f64);
    job.ends_at = later();
    job.max_budget = Some(Decimal::ONE);
    job.spend = Decimal::new(99, 2);
    let jobs = [&job];
    let aliases = BTreeMap::new();
    let starts = BTreeMap::new();
    let just_under = sampled_jobs(&jobs, &sample("req-1"), &starts, &aliases, now);
    job.spend = Decimal::ONE;
    let at_limit = sampled_jobs(&jobs, &sample("req-1"), &starts, &aliases, now);
    assert_eq!(just_under.eligible.len(), 1);
    assert!(at_limit.eligible.is_empty());
}

#[test]
fn shadow_eval_admission_sample_hits_meets_the_bucket_boundary() {
    let boundary = "84.9789919813536".parse::<f64>().unwrap();
    let above = "84.97899198135362".parse::<f64>().unwrap();
    assert!(!sample_hits("req-1", "job-a", boundary));
    assert!(sample_hits("req-1", "job-a", above));
    assert!(!sample_hits("req-1", "job-a", 0.0_f64));
    assert!(sample_hits("req-1", "job-a", 100.0_f64));
    assert_eq!(
        sample_hits("req-1", "job-a", 50.0_f64),
        sample_hits("req-1", "job-a", 50.0_f64)
    );
}

#[test]
fn shadow_eval_admission_sample_hits_hashes_the_job_too() {
    assert!(!sample_hits("req-2", "job-b", 90.0_f64));
    assert!(sample_hits("req-2", "job-a", 90.0_f64));
    assert!(sample_hits("req-1", "job-a", 90.0_f64));
}

#[test]
fn shadow_eval_admission_spend_counter_key_names_the_job() {
    assert_eq!(job_spend_counter_key("job-a"), "spend:shadow_eval:job-a");
}

#[test]
fn shadow_eval_scope_admits_every_group_when_unscoped() {
    let job = active_job("job-a", ShadowEvalDirection::Forward, 100.0_f64);
    let aliases = BTreeMap::new();
    assert!(scope_admits(&job, &aliases, "any-model"));
    assert!(scope_admits(&job, &aliases, ""));
}

#[test]
fn shadow_eval_scope_matches_scope_names_through_the_alias_map() {
    let mut job = active_job("job-a", ShadowEvalDirection::Forward, 100.0_f64);
    job.models = BTreeSet::from(["gpt-4o".to_owned()]);
    let mut aliases = BTreeMap::new();
    aliases.insert("gpt-4o".to_owned(), "gpt-4o-2024".to_owned());
    assert!(scope_admits(&job, &aliases, "gpt-4o-2024"));
    assert!(!scope_admits(&job, &aliases, "gpt-4o"));
    assert!(!scope_admits(&job, &aliases, "gpt-4o-mini"));
}

#[test]
fn shadow_eval_scope_ignores_an_alias_with_an_empty_target() {
    let mut job = active_job("job-a", ShadowEvalDirection::Forward, 100.0_f64);
    job.models = BTreeSet::from(["fast".to_owned()]);
    let mut aliases = BTreeMap::new();
    aliases.insert("fast".to_owned(), String::new());
    assert_eq!(canonical_group(&aliases, "fast"), "fast");
    assert!(scope_admits(&job, &aliases, "fast"));
    assert!(!scope_admits(&job, &aliases, ""));
}

#[test]
fn shadow_eval_scope_alias_rewrites_the_request_group() {
    let mut job = active_job("job-a", ShadowEvalDirection::Forward, 100.0_f64);
    job.ends_at = later();
    job.models = BTreeSet::from(["gpt-4o".to_owned()]);
    let mut aliases = BTreeMap::new();
    aliases.insert("fast".to_owned(), "gpt-4o".to_owned());
    let jobs = [&job];
    let starts = BTreeMap::new();
    let aliased = RequestAdmission {
        routed_by: None,
        model_group: "fast",
        request_id: "req-1",
    };
    let sampled = sampled_jobs(&jobs, &aliased, &starts, &aliases, instant());
    assert_eq!(sampled.eligible.len(), 1);
}

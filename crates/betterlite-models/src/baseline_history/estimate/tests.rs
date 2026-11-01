#![cfg(test)]

use rstest::rstest;

use super::{BaselineEstimate, Provenance, entry_status, estimate_for, is_plan_valid, markers_for};
use crate::baseline_history::cache_plan::{CountedBreakpoint, CountedPromptCachePlan};
use crate::baseline_history::model::{BaselineHistory, BaselineObservation, CacheEntry, Outcome};
use crate::baseline_history::usage::{CacheCreationTokenDetails, PromptTokensDetails, Usage};
use crate::decisions::Extra;

fn breakpoint(fingerprint: &str, ttl_seconds: i64, prefix_tokens: i64) -> CountedBreakpoint {
    CountedBreakpoint {
        fingerprint: fingerprint.to_owned(),
        ttl_seconds,
        prefix_tokens,
        lookback_fingerprints: vec![fingerprint.to_owned()],
        content_fingerprint: fingerprint.to_owned(),
        lookback_content_fingerprints: vec![fingerprint.to_owned()],
    }
}

fn plan(total_tokens: i64, breakpoints: Vec<CountedBreakpoint>) -> CountedPromptCachePlan {
    CountedPromptCachePlan {
        total_tokens,
        breakpoints,
    }
}

fn usage(prompt: i64, completion: i64, text: i64, cached: i64, creation: i64) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt.checked_add(completion).expect("total fits"),
        prompt_tokens_details: Some(PromptTokensDetails {
            text_tokens: Some(text),
            cached_tokens: Some(cached),
            cache_creation_tokens: Some(creation),
            cache_creation_token_details: if creation == 0 {
                None
            } else {
                Some(CacheCreationTokenDetails {
                    ephemeral_5m_input_tokens: Some(creation),
                    ephemeral_1h_input_tokens: Some(0),
                })
            },
            cache_write_tokens: None,
            extra: Extra::default(),
        }),
        extra: Extra::default(),
    }
}

fn observation(
    request_id: &str,
    started_at: f64,
    outcome: Outcome,
    equivalent: bool,
) -> BaselineObservation {
    BaselineObservation {
        request_id: request_id.to_owned(),
        started_at,
        available_at: started_at,
        outcome,
        baseline_equivalent: equivalent,
        usage: None,
        plan: None,
        minimum_cache_tokens: 0,
        reason: None,
    }
}

fn entry(
    fingerprint: &str,
    tokens: i64,
    ttl_seconds: i64,
    available_at: f64,
    expires_at: f64,
) -> CacheEntry {
    CacheEntry {
        fingerprint: fingerprint.to_owned(),
        content_fingerprint: fingerprint.to_owned(),
        tokens,
        ttl_seconds,
        available_at,
        expires_at,
        uncertain: false,
    }
}

#[test]
fn is_plan_valid_accepts_an_ordered_plan() {
    let cache_plan = plan(
        100,
        vec![breakpoint("a", 3600, 10), breakpoint("b", 300, 50)],
    );
    assert!(is_plan_valid(Some(&cache_plan)));
}

#[rstest]
#[case::no_plan(None)]
#[case::too_many_breakpoints(Some(plan(10, vec![
    breakpoint("a", 300, 1), breakpoint("b", 300, 2), breakpoint("c", 300, 3), breakpoint("d", 300, 4), breakpoint("e", 300, 5),
])))]
#[case::negative_total(Some(plan(-1, vec![])))]
#[case::increasing_ttl(Some(plan(100, vec![breakpoint("a", 300, 10), breakpoint("b", 3600, 50)])))]
#[case::prefix_exceeds_total(Some(plan(10, vec![breakpoint("a", 300, 11)])))]
fn is_plan_valid_rejects_every_malformed_shape(#[case] cache_plan: Option<CountedPromptCachePlan>) {
    assert!(!is_plan_valid(cache_plan.as_ref()));
}

#[test]
fn markers_for_drops_breakpoints_below_the_minimum() {
    let cache_plan = plan(100, vec![breakpoint("a", 300, 5), breakpoint("b", 300, 50)]);
    let markers = markers_for(&cache_plan, 10);
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].fingerprint, "b");
}

#[test]
fn entry_status_requires_the_window_the_fingerprint_and_the_ttl_to_match() {
    let fp = breakpoint("fp", 300, 100);
    let markers = vec![&fp];
    let hit = entry("fp", 50, 300, 0.0, 10.0);
    assert!(entry_status(&hit, &markers, 5.0).matches);
    assert!(!entry_status(&hit, &markers, 20.0).matches);
}

#[test]
fn entry_status_flags_a_changed_ttl_on_the_same_content_as_ambiguous() {
    let fp = breakpoint("fp", 3600, 100);
    let markers = vec![&fp];
    let changed_ttl = entry("fp", 50, 300, 0.0, 10.0);
    assert!(entry_status(&changed_ttl, &markers, 5.0).ambiguous);
}

#[test]
fn estimate_for_an_incomplete_observation_falls_back_to_the_outcome_reason() {
    let history = BaselineHistory::default();
    let observation = observation("req-1", 0.0, Outcome::Uncertain, true);
    let estimate = estimate_for(&history, &observation, true).expect("checked estimate");
    assert_eq!(
        estimate,
        BaselineEstimate {
            request_id: "req-1".to_owned(),
            reason: "uncertain".to_owned(),
            provenance: None,
            usage: None
        }
    );
}

#[test]
fn estimate_for_keeps_an_explicit_empty_reason_instead_of_the_outcome_fallback() {
    let history = BaselineHistory::default();
    let mut observation = observation("req-1", 0.0, Outcome::Uncertain, true);
    observation.reason = Some(String::new());
    let estimate = estimate_for(&history, &observation, true).expect("checked estimate");
    assert_eq!(estimate.reason, "");
}

#[test]
fn estimate_for_an_identical_path_copies_usage_without_modeling() {
    let history = BaselineHistory::default();
    let mut observation = observation("req-1", 0.0, Outcome::Complete, true);
    observation.usage = Some(usage(10, 1, 10, 0, 0));
    let estimate = estimate_for(&history, &observation, true).expect("checked estimate");
    assert_eq!(estimate.provenance, Some(Provenance::ObservedIdentical));
    assert_eq!(estimate.reason, "identical_baseline_path");
}

#[test]
fn estimate_for_a_blocked_request_reports_concurrent_uncertainty() {
    let history = BaselineHistory {
        first_at: None,
        last_at: None,
        equivalent: true,
        uncertain_before: 0.0_f64,
        entries: Vec::new(),
        blocked_until: 100.0_f64,
    };
    let mut observation = observation("req-1", 10.0, Outcome::Complete, false);
    observation.usage = Some(usage(10, 1, 10, 0, 0));
    let estimate = estimate_for(&history, &observation, false).expect("checked estimate");
    assert_eq!(estimate.reason, "concurrent_uncertainty");
}

#[test]
fn estimate_for_a_cold_cache_models_zero_read_tokens() {
    let history = BaselineHistory::default();
    let mut observation = observation("req-1", 10_000.0, Outcome::Complete, false);
    observation.usage = Some(usage(100, 1, 100, 0, 0));
    observation.plan = Some(plan(100, vec![breakpoint("a", 3600, 80)]));
    let estimate = estimate_for(&history, &observation, false).expect("checked estimate");
    assert_eq!(estimate.reason, "cache_prefix_cold");
    assert_eq!(estimate.provenance, Some(Provenance::Modeled));
    let usage = estimate.usage.expect("modeled usage");
    assert_eq!(
        usage.prompt_tokens_details.expect("details").cached_tokens,
        Some(0)
    );
}

#[test]
fn estimate_for_a_cache_hit_reads_the_matching_entry() {
    let mut history = BaselineHistory::default();
    history.entries.push(entry("a", 80, 3600, 0.0, 3600.0));
    let mut observation = observation("req-1", 10.0, Outcome::Complete, false);
    observation.usage = Some(usage(100, 1, 100, 0, 0));
    observation.plan = Some(plan(100, vec![breakpoint("a", 3600, 80)]));
    let estimate = estimate_for(&history, &observation, false).expect("checked estimate");
    assert_eq!(estimate.reason, "cache_prefix_available");
    let usage = estimate.usage.expect("modeled usage");
    assert_eq!(
        usage.prompt_tokens_details.expect("details").cached_tokens,
        Some(80)
    );
}

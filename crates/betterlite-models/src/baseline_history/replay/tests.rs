#![cfg(test)]

use std::cmp::Ordering;

use super::advance_baseline_history;
use crate::baseline_history::cache_plan::{CountedBreakpoint, CountedPromptCachePlan};
use crate::baseline_history::error::BaselineHistoryError;
use crate::baseline_history::model::{BaselineHistory, BaselineObservation, CacheEntry, Outcome};
use crate::baseline_history::usage::{PromptTokensDetails, Usage};
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

fn usage(prompt: i64, completion: i64) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt.checked_add(completion).expect("total fits"),
        prompt_tokens_details: Some(PromptTokensDetails {
            text_tokens: Some(prompt),
            cached_tokens: Some(0),
            cache_creation_tokens: Some(0),
            cache_creation_token_details: None,
            extra: Extra::default(),
        }),
        extra: Extra::default(),
    }
}

fn assert_timestamp(actual: f64, expected: f64) {
    assert_eq!(actual.total_cmp(&expected), Ordering::Equal);
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

#[test]
fn an_empty_group_leaves_the_history_untouched() {
    let history = BaselineHistory::default();
    let (next, estimates) = advance_baseline_history(&history, &[]).expect("checked replay");
    assert_eq!(next, history);
    assert!(estimates.is_empty());
}

#[test]
fn a_timestamp_that_does_not_advance_is_rejected_without_mutating_the_history() {
    let history = BaselineHistory {
        first_at: None,
        last_at: Some(10.0_f64),
        equivalent: true,
        uncertain_before: 0.0_f64,
        entries: Vec::new(),
        blocked_until: 0.0_f64,
    };
    let group = [observation("req-1", 5.0, Outcome::Complete, true)];
    let (next, estimates) = advance_baseline_history(&history, &group).expect("checked replay");
    assert_eq!(next, history);
    assert_eq!(estimates[0].reason, "invalid_observation_order");
}

#[test]
fn an_invalid_observation_blocks_every_relevant_observation_in_its_group() {
    let history = BaselineHistory::default();
    let mut valid = observation("req-1", 10.0, Outcome::Complete, false);
    valid.usage = Some(usage(100, 1));
    valid.plan = Some(CountedPromptCachePlan {
        total_tokens: 100,
        breakpoints: vec![breakpoint("a", 3600, 80)],
    });
    valid.available_at = 15.0_f64;
    let mut invalid = observation("req-2", 10.0, Outcome::Complete, false);
    invalid.available_at = 20.0_f64;
    let group = [valid, invalid];
    let (next, _estimates) = advance_baseline_history(&history, &group).expect("checked replay");
    assert_timestamp(next.blocked_until, 20.0_f64);
    assert!(next.entries.is_empty());
}

#[test]
fn a_response_cache_observation_does_not_invalidate_the_history() {
    let mut history = BaselineHistory::default();
    history.entries.push(CacheEntry {
        fingerprint: "a".to_owned(),
        content_fingerprint: "a".to_owned(),
        tokens: 80,
        ttl_seconds: 3600,
        available_at: 0.0,
        expires_at: 3600.0,
        uncertain: false,
    });
    let cached = observation("req-1", 10.0, Outcome::ResponseCache, false);
    let (next, _estimates) = advance_baseline_history(&history, &[cached]).expect("checked replay");
    assert_eq!(next.entries.len(), 1);
    assert_timestamp(next.blocked_until, 0.0_f64);
}

#[test]
fn overflowing_the_entry_cap_clears_history_and_marks_it_uncertain() {
    let history = BaselineHistory {
        first_at: None,
        last_at: None,
        equivalent: true,
        uncertain_before: 0.0_f64,
        entries: (0_i32..1_025_i32)
            .map(|index| CacheEntry {
                fingerprint: format!("fp-{index}"),
                content_fingerprint: format!("fp-{index}"),
                tokens: 1,
                ttl_seconds: 3600,
                available_at: 0.0,
                expires_at: 3600.0,
                uncertain: false,
            })
            .collect(),
        blocked_until: 0.0_f64,
    };
    let observation = observation("req-1", 10.0, Outcome::ResponseCache, false);
    let (next, _estimates) =
        advance_baseline_history(&history, &[observation]).expect("checked replay");
    assert!(next.entries.is_empty());
    assert_timestamp(next.uncertain_before, 10.0_f64);
}

#[test]
fn an_already_infinite_history_timestamp_surfaces_the_checked_arithmetic_deviation() {
    let history = BaselineHistory {
        first_at: None,
        last_at: None,
        equivalent: true,
        uncertain_before: f64::INFINITY,
        entries: Vec::new(),
        blocked_until: 0.0_f64,
    };
    let mut observation = observation("req-1", 10.0, Outcome::Complete, false);
    observation.usage = Some(usage(100, 1));
    observation.plan = Some(CountedPromptCachePlan {
        total_tokens: 100,
        breakpoints: vec![breakpoint("a", 3600, 80)],
    });
    let error = advance_baseline_history(&history, &[observation])
        .expect_err("uncertain_before is already infinite");
    assert_eq!(error, BaselineHistoryError::TimestampOutOfRange);
}

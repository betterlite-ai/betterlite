#![cfg(test)]

use std::cmp::Ordering;

use super::{compact_entries, writes_for};
use crate::baseline_history::cache_plan::{CountedBreakpoint, CountedPromptCachePlan};
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
            cache_write_tokens: None,
            extra: Extra::default(),
        }),
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        extra: Extra::default(),
    }
}

fn observation(request_id: &str, started_at: f64) -> BaselineObservation {
    BaselineObservation {
        request_id: request_id.to_owned(),
        started_at,
        available_at: started_at,
        outcome: Outcome::Complete,
        baseline_equivalent: false,
        usage: Some(usage(100, 1)),
        plan: Some(CountedPromptCachePlan {
            total_tokens: 100,
            breakpoints: vec![breakpoint("a", 3600, 80)],
        }),
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
fn writes_for_a_cold_plan_writes_one_entry_per_marker() {
    let history = BaselineHistory::default();
    let observation = observation("req-1", 0.0);
    let entries = writes_for(&history, &observation).expect("checked writes");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].fingerprint, "a");
    assert_eq!(
        entries[0].expires_at.total_cmp(&3600.0_f64),
        Ordering::Equal
    );
}

#[test]
fn writes_for_an_incomplete_outcome_writes_nothing() {
    let history = BaselineHistory::default();
    let mut observation = observation("req-1", 0.0);
    observation.outcome = Outcome::Uncertain;
    let entries = writes_for(&history, &observation).expect("checked writes");
    assert!(entries.is_empty());
}

#[test]
fn writes_for_emits_a_refresh_before_the_marker_entry() {
    let mut history = BaselineHistory::default();
    history.entries.push(entry("stale", 50, 3600, 0.0, 10.0));
    let mut observation = observation("req-1", 5.0);
    observation.plan = Some(CountedPromptCachePlan {
        total_tokens: 100,
        breakpoints: vec![CountedBreakpoint {
            fingerprint: "fresh".to_owned(),
            ttl_seconds: 3600,
            prefix_tokens: 80,
            lookback_fingerprints: vec!["stale".to_owned(), "fresh".to_owned()],
            content_fingerprint: "fresh".to_owned(),
            lookback_content_fingerprints: vec!["fresh".to_owned()],
        }],
    });
    let entries = writes_for(&history, &observation).expect("checked writes");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].fingerprint, "stale");
    assert_eq!(entries[0].available_at.total_cmp(&5.0_f64), Ordering::Equal);
    assert_eq!(
        entries[0].expires_at.total_cmp(&3605.0_f64),
        Ordering::Equal
    );
    assert_eq!(entries[1].fingerprint, "fresh");
}

#[test]
fn writes_for_a_request_started_before_blocked_until_writes_nothing() {
    let history = BaselineHistory {
        blocked_until: 100.0,
        first_at: None,
        last_at: None,
        equivalent: true,
        uncertain_before: 0.0,
        entries: Vec::new(),
    };
    let observation = observation("req-1", 50.0);
    assert!(
        writes_for(&history, &observation)
            .expect("checked writes")
            .is_empty()
    );
}

#[test]
fn writes_for_an_ambiguous_write_expires_at_the_longest_conflicting_ttl() {
    let mut history = BaselineHistory::default();
    let mut old = entry("old", 20, 3600, 0.0, 3600.0);
    old.content_fingerprint = "c".to_owned();
    history.entries.push(old);
    let mut observation = observation("req-1", 5.0);
    let mut marker = breakpoint("new", 300, 80);
    marker.content_fingerprint = "c".to_owned();
    marker.lookback_content_fingerprints = vec!["c".to_owned()];
    observation.plan = Some(CountedPromptCachePlan {
        total_tokens: 100,
        breakpoints: vec![marker],
    });
    let entries = writes_for(&history, &observation).expect("checked writes");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].uncertain);
    assert_eq!(entries[0].ttl_seconds, 300);
    assert_eq!(
        entries[0].expires_at.total_cmp(&3605.0_f64),
        Ordering::Equal
    );
}

#[test]
fn compact_entries_keeps_the_longest_lived_duplicate_and_every_future_entry() {
    let entries = vec![
        entry("a", 10, 300, 0.0, 300.0),
        entry("a", 10, 300, 0.0, 600.0),
        entry("a", 10, 300, 50.0, 350.0),
    ];
    let compacted = compact_entries(&entries, 10.0);
    assert_eq!(compacted.len(), 2);
    assert!(
        compacted
            .iter()
            .any(|entry| entry.expires_at.total_cmp(&600.0_f64) == Ordering::Equal)
    );
    assert!(
        compacted
            .iter()
            .any(|entry| entry.available_at.total_cmp(&50.0_f64) == Ordering::Equal)
    );
}

#[test]
fn compact_entries_drops_entries_that_expired_past_the_retention_window() {
    let entries = vec![entry("a", 10, 300, 0.0, 10.0)];
    let compacted = compact_entries(&entries, 10_000.0);
    assert!(compacted.is_empty());
}

#[test]
fn compact_entries_orders_by_fingerprint_key_not_by_insertion() {
    let entries = vec![
        entry("zeta", 10, 300, 0.0, 300.0),
        entry("alpha", 10, 300, 0.0, 300.0),
    ];
    let compacted = compact_entries(&entries, 10.0);
    assert_eq!(
        compacted
            .iter()
            .map(|entry| entry.fingerprint.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );
}

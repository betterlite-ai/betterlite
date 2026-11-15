use std::time::Duration;

use rstest::rstest;

use crate::compresr_recovery::retrieval::{MAX_RETRIEVALS_PER_LOOP, resolve_retrievals};
use crate::compresr_recovery::store::ORIGINALS_TTL;
use crate::compresr_recovery::tools::RetrieveCall;

use super::support::{CALL_ID, NOW, SCOPE, key, retrieve_call, seeded};

const MISSING: &str = "[compresr: hash=nope not found, expired, or not issued for this request]";

fn contents(calls: Vec<RetrieveCall>, pairs: &[(&str, &str)]) -> (Vec<String>, bool) {
    let store = seeded(pairs);
    let resolution = resolve_retrievals(&store, Some(&key(SCOPE, CALL_ID)), calls, NOW);
    let texts = resolution
        .retrieved
        .into_iter()
        .map(|entry| entry.content)
        .collect();
    (texts, resolution.resolved_any)
}

#[test]
fn stored_original_is_returned_in_call_order_retrieval() {
    let calls = vec![retrieve_call("c1", "h2"), retrieve_call("c2", "h1")];
    let (texts, resolved) = contents(calls, &[("h1", "first"), ("h2", "second")]);
    assert_eq!(texts, ["second", "first"]);
    assert!(resolved);
}

#[test]
fn repeated_hash_answers_with_a_repeat_notice_retrieval() {
    let calls = vec![retrieve_call("c1", "h1"), retrieve_call("c2", "h1")];
    let (texts, _) = contents(calls, &[("h1", "first")]);
    assert_eq!(
        texts,
        ["first", "[compresr: already retrieved above for this hash]"]
    );
}

#[test]
fn ninth_call_answers_with_the_limit_notice_retrieval() {
    assert_eq!(MAX_RETRIEVALS_PER_LOOP, 8);
    let hashes: Vec<String> = (0_usize..9).map(|index| format!("h{index}")).collect();
    let pairs: Vec<(&str, &str)> = hashes.iter().map(|hash| (hash.as_str(), "text")).collect();
    let calls = hashes.iter().map(|hash| retrieve_call("c", hash)).collect();
    let (texts, _) = contents(calls, &pairs);
    assert!(texts.iter().take(8).all(|text| text == "text"));
    assert_eq!(
        texts[8],
        "[compresr: retrieval limit reached for this turn]"
    );
}

#[test]
fn limit_notice_wins_over_the_repeat_notice_retrieval() {
    let mut calls: Vec<RetrieveCall> = (0_usize..8)
        .map(|index| retrieve_call("c", &format!("h{index}")))
        .collect();
    calls.push(retrieve_call("c", "h0"));
    let pairs: Vec<(String, &str)> = (0_usize..8)
        .map(|index| (format!("h{index}"), "text"))
        .collect();
    let refs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(hash, text)| (hash.as_str(), *text))
        .collect();
    let (texts, _) = contents(calls, &refs);
    assert_eq!(
        texts[8],
        "[compresr: retrieval limit reached for this turn]"
    );
}

#[test]
fn unknown_hash_falls_back_to_a_notice_retrieval() {
    let (texts, resolved) = contents(vec![retrieve_call("c1", "nope")], &[("h1", "first")]);
    assert_eq!(texts, [MISSING]);
    assert!(!resolved);
}

#[test]
fn repeated_missing_hash_is_not_deduplicated_retrieval() {
    let calls = vec![retrieve_call("c1", "nope"), retrieve_call("c2", "nope")];
    let (texts, _) = contents(calls, &[("h1", "first")]);
    assert_eq!(texts, [MISSING, MISSING]);
}

#[rstest]
#[case::other_caller(Some(("scope-b", CALL_ID)), NOW)]
#[case::other_call_id(Some((SCOPE, "call-2")), NOW)]
#[case::no_key(None, NOW)]
#[case::expired(Some((SCOPE, CALL_ID)), NOW.saturating_add(ORIGINALS_TTL))]
fn hash_out_of_scope_or_expired_falls_back_retrieval(
    #[case] scope_and_call: Option<(&str, &str)>,
    #[case] now: Duration,
) {
    let store = seeded(&[("h1", "first")]);
    let lookup = scope_and_call.map(|(scope, call_id)| key(scope, call_id));
    let calls = vec![retrieve_call("c1", "h1")];
    let resolution = resolve_retrievals(&store, lookup.as_ref(), calls, now);
    assert_eq!(
        resolution.retrieved[0].content,
        "[compresr: hash=h1 not found, expired, or not issued for this request]"
    );
    assert!(!resolution.resolved_any);
}

#[test]
fn fallback_notice_shows_a_bounded_printable_hash_retrieval() {
    let hostile = format!("{}\u{1b}[31m{}", "a".repeat(20), "b".repeat(20));
    let (texts, _) = contents(vec![retrieve_call("c1", &hostile)], &[]);
    let shown = format!("{}[31m{}\u{2026}", "a".repeat(20), "b".repeat(8));
    assert_eq!(
        texts,
        [format!(
            "[compresr: hash={shown} not found, expired, or not issued for this request]"
        )]
    );
}

#[test]
fn non_text_hash_argument_is_looked_up_as_empty_retrieval() {
    let call: RetrieveCall =
        serde_json::from_str(r#"{"id":"c1","name":"compresr_retrieve","arguments":{"hash":5}}"#)
            .unwrap();
    let (texts, resolved) = contents(vec![call], &[("h1", "first")]);
    assert_eq!(
        texts,
        ["[compresr: hash= not found, expired, or not issued for this request]"]
    );
    assert!(!resolved);
}

#[test]
fn one_resolved_call_marks_the_plan_runnable_retrieval() {
    let calls = vec![retrieve_call("c1", "nope"), retrieve_call("c2", "h1")];
    let (texts, resolved) = contents(calls, &[("h1", "first")]);
    assert_eq!(texts, [MISSING, "first"]);
    assert!(resolved);
}

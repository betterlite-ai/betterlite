use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::repello_lifecycle::BLOCKED_DETAIL;
use betterlite_guardrails::repello_lifecycle::verdict::{format_blocked_detail, raise_if_blocked};
use rstest::rstest;

use crate::support::parsed;

#[rstest]
#[case::no_policies(r#"{"verdict": "blocked"}"#)]
#[case::empty_policies(r#"{"verdict": "blocked", "policies_violated": []}"#)]
#[case::null_policies(r#"{"verdict": "blocked", "policies_violated": null}"#)]
fn a_block_without_policies_uses_the_default_message(#[case] reply: &str) {
    assert_eq!(format_blocked_detail(&parsed(reply)), BLOCKED_DETAIL);
    assert_eq!(BLOCKED_DETAIL, "Blocked by RepelloAI Argus guardrail.");
}

#[rstest]
#[case::named_only(
    r#"{"verdict": "blocked", "policies_violated": [{"policy_name": "pii"}]}"#,
    "Blocked by RepelloAI Argus guardrail. Policies violated: pii."
)]
#[case::no_name(
    r#"{"verdict": "blocked", "policies_violated": [{}]}"#,
    "Blocked by RepelloAI Argus guardrail. Policies violated: unknown_policy."
)]
#[case::action(
    r#"{"verdict": "blocked", "policies_violated": [{"policy_name": "pii", "action_taken": "block"}]}"#,
    "Blocked by RepelloAI Argus guardrail. Policies violated: pii (action: block)."
)]
#[case::number_score(
    r#"{"verdict": "blocked", "policies_violated": [{"policy_name": "pii", "details": {"score": 0.9}}]}"#,
    "Blocked by RepelloAI Argus guardrail. Policies violated: pii (score: 0.9)."
)]
#[case::integer_score(
    r#"{"verdict": "blocked", "policies_violated": [{"policy_name": "pii", "details": {"score": 9}}]}"#,
    "Blocked by RepelloAI Argus guardrail. Policies violated: pii (score: 9)."
)]
#[case::text_score(
    r#"{"verdict": "blocked", "policies_violated": [{"policy_name": "pii", "details": {"score": "high"}}]}"#,
    "Blocked by RepelloAI Argus guardrail. Policies violated: pii (score: high)."
)]
#[case::action_and_score(
    r#"{"verdict": "blocked", "policies_violated": [{"policy_name": "pii", "action_taken": "block", "details": {"score": 0.5}}]}"#,
    "Blocked by RepelloAI Argus guardrail. Policies violated: pii (action: block, score: 0.5)."
)]
#[case::several_policies(
    r#"{"verdict": "blocked", "policies_violated": [{"policy_name": "pii", "action_taken": "block"}, {"policy_name": "toxicity", "details": {"score": 0.25}}]}"#,
    "Blocked by RepelloAI Argus guardrail. Policies violated: pii (action: block); toxicity (score: 0.25)."
)]
#[case::other_detail_keys_are_ignored(
    r#"{"verdict": "blocked", "policies_violated": [{"policy_name": "pii", "details": {"reason": "x"}}]}"#,
    "Blocked by RepelloAI Argus guardrail. Policies violated: pii."
)]
fn the_blocked_detail_matches_litellm(#[case] reply: &str, #[case] expected: &str) {
    assert_eq!(format_blocked_detail(&parsed(reply)), expected);
}

#[test]
fn a_blocked_response_is_a_400_with_the_detail() {
    let error = raise_if_blocked(Some(&parsed(
        r#"{"verdict": "blocked", "policies_violated": [{"policy_name": "pii"}]}"#,
    )))
    .unwrap_err();
    assert!(matches!(
        error,
        GuardrailFailure::Http { status: 400, ref detail }
            if detail.as_str() == "Blocked by RepelloAI Argus guardrail. Policies violated: pii."
    ));
}

#[test]
fn no_response_is_not_blocked() {
    assert!(raise_if_blocked(None).is_ok());
}

#[rstest]
#[case::passed(r#"{"verdict": "passed"}"#)]
#[case::flagged(r#"{"verdict": "flagged"}"#)]
fn a_non_blocking_response_passes_through(#[case] reply: &str) {
    assert!(raise_if_blocked(Some(&parsed(reply))).is_ok());
}

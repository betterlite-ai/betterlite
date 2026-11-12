use super::support::{json, thinking_object};
use crate::capabilities::budget::cap_thinking_to_max_tokens;

fn cap_absent_max_tokens_is_unchanged() {
    let display = r#"{"type": "enabled", "budget_tokens": 5000, "display": "summarized"}"#;
    assert_eq!(
        cap_thinking_to_max_tokens(&thinking_object(display), None),
        Some(json(display)),
        "an absent max_tokens returns the caller's dict untouched"
    );
}

fn cap_at_the_floor_drops_thinking() {
    assert_eq!(
        cap_thinking_to_max_tokens(
            &thinking_object(r#"{"type": "enabled", "budget_tokens": 100}"#),
            Some(1024)
        ),
        None,
        "max_tokens at the floor drops thinking even when the budget fits"
    );
    assert_eq!(
        cap_thinking_to_max_tokens(
            &thinking_object(r#"{"type": "enabled", "budget_tokens": 5000}"#),
            Some(-5)
        ),
        None
    );
}

fn cap_keeps_a_fitting_budget() {
    let fitting = r#"{"type": "enabled", "budget_tokens": 1024}"#;
    assert_eq!(
        cap_thinking_to_max_tokens(&thinking_object(fitting), Some(1025)),
        Some(json(fitting)),
        "a budget below max_tokens is forwarded unchanged"
    );
}

fn cap_rebuilds_only_type_and_budget() {
    let display = r#"{"type": "enabled", "budget_tokens": 5000, "display": "summarized"}"#;
    assert_eq!(
        cap_thinking_to_max_tokens(&thinking_object(display), Some(1025)),
        Some(json(r#"{"type": "enabled", "budget_tokens": 1024}"#)),
        "the cap keeps only type and the new budget"
    );
    assert_eq!(
        cap_thinking_to_max_tokens(
            &thinking_object(
                r#"{"type": "adaptive", "budget_tokens": 9000, "display": "omitted", "extra": 1}"#
            ),
            Some(2048)
        ),
        Some(json(r#"{"type": "adaptive", "budget_tokens": 2047}"#)),
        "capping drops display and every other field"
    );
}

fn cap_treats_equal_budget_as_too_large() {
    assert_eq!(
        cap_thinking_to_max_tokens(
            &thinking_object(r#"{"type": "enabled", "budget_tokens": 1025}"#),
            Some(1025)
        ),
        Some(json(r#"{"type": "enabled", "budget_tokens": 1024}"#)),
        "Anthropic requires max_tokens > budget_tokens"
    );
}

fn cap_defaults_an_absent_type() {
    assert_eq!(
        cap_thinking_to_max_tokens(&thinking_object(r#"{"budget_tokens": 5000}"#), Some(2048)),
        Some(json(r#"{"type": "enabled", "budget_tokens": 2047}"#)),
        "an absent type defaults to enabled"
    );
}

fn cap_leaves_noninteger_budgets_alone() {
    let fractional = r#"{"type": "enabled", "budget_tokens": 5000.5}"#;
    assert_eq!(
        cap_thinking_to_max_tokens(&thinking_object(fractional), Some(2048)),
        Some(json(fractional))
    );
    let boolean = r#"{"type": "enabled", "budget_tokens": true}"#;
    assert_eq!(
        cap_thinking_to_max_tokens(&thinking_object(boolean), Some(2048)),
        Some(json(boolean)),
        "a boolean budget is an int in Python and fits below max_tokens"
    );
    let absent = r#"{"type": "enabled"}"#;
    assert_eq!(
        cap_thinking_to_max_tokens(&thinking_object(absent), Some(2048)),
        Some(json(absent))
    );
    let null_budget = r#"{"type": "enabled", "budget_tokens": null}"#;
    assert_eq!(
        cap_thinking_to_max_tokens(&thinking_object(null_budget), Some(2048)),
        Some(json(null_budget))
    );
}

pub(super) fn max_token_budget_cap() {
    cap_absent_max_tokens_is_unchanged();
    cap_at_the_floor_drops_thinking();
    cap_keeps_a_fitting_budget();
    cap_rebuilds_only_type_and_budget();
    cap_treats_equal_budget_as_too_large();
    cap_defaults_an_absent_type();
    cap_leaves_noninteger_budgets_alone();
}

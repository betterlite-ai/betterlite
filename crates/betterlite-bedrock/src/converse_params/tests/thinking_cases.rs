use crate::converse_params::budget::update_thinking_budget;

use super::scenario::{ANTHROPIC, DROP_CALL, DROP_GLOBAL, Harness, KEEP, MODEL_COST};
use super::support::params;

const ENABLED_BUDGET: &str = r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#;

pub(super) fn thinking_budget_with_max_tokens() {
    disabled_budget_clamped();
    enabled_budget_fills_max_tokens();
    requested_max_tokens_keep_budget();
    numeric_budget_totals();
    unusable_budget_values();
    budget_skipped_without_thinking();
    budget_kept_on_reasoning_models();
    anthropic_thinking_budget();
}

fn disabled_budget_clamped() {
    let mut disabled =
        params(r#"{"thinking": {"type": "disabled", "budget_tokens": 100}, "maxTokens": 5}"#);
    update_thinking_budget(&params("{}"), &mut disabled, 1024).expect("the budget updates");
    assert_eq!(
        disabled,
        params(r#"{"thinking": {"type": "disabled", "budget_tokens": 1024}, "maxTokens": 5}"#)
    );
}

fn enabled_budget_fills_max_tokens() {
    let mut without_max = params(ENABLED_BUDGET);
    update_thinking_budget(&params("{}"), &mut without_max, 1024).expect("the budget updates");
    assert_eq!(
        without_max,
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "maxTokens": 6144}"#)
    );
    let mut with_small_max =
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "maxTokens": 999}"#);
    update_thinking_budget(&params("{}"), &mut with_small_max, 1024).expect("the budget updates");
    assert_eq!(
        with_small_max,
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "maxTokens": 6144}"#)
    );
}

fn requested_max_tokens_keep_budget() {
    for non_default in [
        r#"{"max_tokens": null}"#,
        r#"{"max_completion_tokens": null}"#,
        r#"{"max_tokens": 100}"#,
    ] {
        let mut optional = params(ENABLED_BUDGET);
        update_thinking_budget(&params(non_default), &mut optional, 1024)
            .expect("the budget updates");
        assert_eq!(optional, params(ENABLED_BUDGET), "{non_default}");
    }
}

fn numeric_budget_totals() {
    let mut fractional = params(r#"{"thinking": {"type": "enabled", "budget_tokens": 1024.5}}"#);
    update_thinking_budget(&params("{}"), &mut fractional, 1024).expect("the budget updates");
    assert_eq!(
        fractional,
        params(
            r#"{"thinking": {"type": "enabled", "budget_tokens": 1024.5}, "maxTokens": 5120.5}"#
        )
    );
    let mut zero = params(r#"{"thinking": {"type": "enabled", "budget_tokens": 0}}"#);
    update_thinking_budget(&params("{}"), &mut zero, 1024).expect("the budget updates");
    assert_eq!(
        zero,
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 1024}, "maxTokens": 5120}"#)
    );
    for (budget, total) in [("true", 4097_i32), ("false", 4096_i32)] {
        let mut flag = params(&format!(
            r#"{{"thinking": {{"type": "enabled", "budget_tokens": {budget}}}}}"#
        ));
        update_thinking_budget(&params("{}"), &mut flag, 0).expect("the budget updates");
        assert_eq!(
            flag,
            params(&format!(
                r#"{{"thinking": {{"type": "enabled", "budget_tokens": {budget}}}, "maxTokens": {total}}}"#
            )),
            "{budget}"
        );
    }
}

fn unusable_budget_values() {
    let mut absent = params(r#"{"thinking": {"type": "enabled", "budget_tokens": null}}"#);
    update_thinking_budget(&params("{}"), &mut absent, 1024).expect("the budget updates");
    assert_eq!(
        absent,
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": null}}"#)
    );
    let mut textual = params(r#"{"thinking": {"type": "enabled", "budget_tokens": "many"}}"#);
    let error = update_thinking_budget(&params("{}"), &mut textual, 1024)
        .expect_err("a text budget is not a number");
    assert_eq!(error.to_string(), "thinking.budget_tokens must be a number");
}

fn budget_skipped_without_thinking() {
    for source in [
        r#"{"reasoning_effort": "high"}"#,
        r#"{"thinking": true}"#,
        "{}",
    ] {
        let mut optional = params(source);
        update_thinking_budget(&params("{}"), &mut optional, 1024).expect("the budget updates");
        assert_eq!(optional, params(source), "{source}");
    }
}

fn budget_kept_on_reasoning_models() {
    let harness = Harness::new(MODEL_COST);
    for model in ["openai.gpt-oss-120b-1:0", "amazon.nova-2-lite-v1:0"] {
        assert_eq!(
            harness.mapped(model, ENABLED_BUDGET).mapped(),
            params(ENABLED_BUDGET),
            "{model}"
        );
    }
}

fn anthropic_thinking_budget() {
    let harness = Harness::new(MODEL_COST);
    for (request, expected) in [
        (
            r#"{"thinking": {"type": "enabled", "budget_tokens": 100}, "max_tokens": 9000}"#,
            r#"{"maxTokens": 9000, "thinking": {"type": "enabled", "budget_tokens": 1024}}"#,
        ),
        (
            r#"{"thinking": {"type": "enabled", "budget_tokens": 100}}"#,
            r#"{"thinking": {"type": "enabled", "budget_tokens": 1024}, "maxTokens": 5120}"#,
        ),
        (
            r#"{"thinking": {"type": "adaptive"}, "max_completion_tokens": 0, "max_tokens": 1200}"#,
            r#"{"maxTokens": 1200, "thinking": {"type": "enabled", "budget_tokens": 1199}}"#,
        ),
        (
            r#"{"max_completion_tokens": 2000, "max_tokens": 1200, "thinking": {"type": "adaptive"}}"#,
            r#"{"maxTokens": 1200, "thinking": {"type": "enabled", "budget_tokens": 1999}}"#,
        ),
    ] {
        assert_eq!(
            harness.mapped(ANTHROPIC, request).mapped(),
            params(expected),
            "{request}"
        );
    }
}

pub(super) fn adaptive_thinking_fallback() {
    let harness = Harness::new(MODEL_COST);
    adaptive_budget_from_max_tokens(&harness);
    adaptive_dropped_when_too_small(&harness);
    adaptive_budget_errors_and_passthrough(&harness);
}

fn adaptive_budget_from_max_tokens(harness: &Harness) {
    for (max_tokens, budget) in [(3000_i32, 2048_i32), (1500_i32, 1499_i32)] {
        assert_eq!(
            harness
                .mapped(
                    ANTHROPIC,
                    &format!(
                        r#"{{"thinking": {{"type": "adaptive"}}, "max_tokens": {max_tokens}}}"#
                    )
                )
                .mapped(),
            params(&format!(
                r#"{{"maxTokens": {max_tokens}, "thinking": {{"type": "enabled", "budget_tokens": {budget}}}}}"#
            )),
            "{max_tokens}"
        );
    }
    assert_eq!(
        harness
            .mapped(ANTHROPIC, r#"{"thinking": {"type": "adaptive"}}"#)
            .mapped(),
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "maxTokens": 6144}"#)
    );
    assert_eq!(
        harness
            .mapped(
                ANTHROPIC,
                r#"{"thinking": {"type": "adaptive"}, "max_tokens": null}"#
            )
            .mapped(),
        params(r#"{"maxTokens": null, "thinking": {"type": "enabled", "budget_tokens": 2048}}"#)
    );
}

fn adaptive_dropped_when_too_small(harness: &Harness) {
    let warned = harness.mapped(
        ANTHROPIC,
        r#"{"thinking": {"type": "adaptive"}, "max_tokens": 1000}"#,
    );
    assert_eq!(
        warned.warnings(),
        [
            "Dropping adaptive `thinking` for model=anthropic.claude-sonnet-4-20250514-v1:0: max_tokens is too small to fit the minimum thinking budget."
        ]
    );
    assert_eq!(warned.mapped(), params(r#"{"maxTokens": 1000}"#));
    let kept = harness.call(
        ANTHROPIC,
        params(r#"{"thinking": {"type": "adaptive"}, "max_tokens": 500}"#),
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 5}}"#),
        KEEP,
    );
    assert_eq!(kept.warnings().len(), 1);
    assert_eq!(
        kept.mapped(),
        params(r#"{"maxTokens": 500, "thinking": {"type": "enabled", "budget_tokens": 1024}}"#)
    );
}

fn adaptive_budget_errors_and_passthrough(harness: &Harness) {
    let error = harness
        .mapped(
            ANTHROPIC,
            r#"{"thinking": {"type": "adaptive"}, "max_tokens": "900"}"#,
        )
        .error();
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(error.to_string(), "max_tokens must be a number");
    assert_eq!(
        harness
            .mapped(
                ANTHROPIC,
                r#"{"thinking": {"type": "enabled", "budget_tokens": 5000}, "max_tokens": 6000}"#
            )
            .mapped(),
        params(r#"{"maxTokens": 6000, "thinking": {"type": "enabled", "budget_tokens": 5000}}"#)
    );
    assert_eq!(
        harness
            .mapped(
                "anthropic.claude-adaptive",
                r#"{"thinking": {"type": "enabled", "budget_tokens": 4096}, "max_tokens": 9000}"#
            )
            .mapped(),
        params(
            r#"{"maxTokens": 9000, "thinking": {"type": "adaptive"}, "output_config": {"effort": "high"}}"#
        )
    );
}

pub(super) fn final_forced_tool_downgrade() {
    let harness = Harness::new(MODEL_COST);
    forced_tool_choice_downgraded(&harness);
    tool_choice_cases(&harness);
    none_tool_choice_dropped_or_refused(&harness);
}

fn forced_tool_choice_downgraded(harness: &Harness) {
    let downgraded = harness.call(
        ANTHROPIC,
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "max_tokens": 3000}"#),
        params(r#"{"tool_choice": {"any": {}}}"#),
        KEEP,
    );
    assert_eq!(
        downgraded.infos(),
        [
            "anthropic.claude-sonnet-4-20250514-v1:0 does not support forced tool use (tool_choice='required' or specific tool) when reasoning is enabled. Changing tool_choice to 'auto'."
        ]
    );
    assert_eq!(
        downgraded.mapped(),
        params(
            r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "maxTokens": 3000, "tool_choice": {"auto": {}}}"#
        )
    );
}

fn tool_choice_cases(harness: &Harness) {
    for (model, non_default, optional, expected) in [
        (
            ANTHROPIC,
            ENABLED_BUDGET,
            r#"{"tool_choice": {"tool": {"name": "lookup"}}}"#,
            r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "maxTokens": 6144, "tool_choice": {"auto": {}}}"#,
        ),
        (
            "openai.gpt-oss-120b-1:0",
            r#"{"reasoning_effort": "high"}"#,
            r#"{"tool_choice": {"any": {}}}"#,
            r#"{"reasoning_effort": "high", "tool_choice": {"auto": {}}}"#,
        ),
    ] {
        let outcome = harness.call(model, params(non_default), params(optional), KEEP);
        assert_eq!(outcome.mapped(), params(expected), "{model}");
    }
    let thinking = ENABLED_BUDGET;
    for (non_default, optional) in [
        (thinking, r#"{"tool_choice": {"auto": {}}}"#),
        ("{}", r#"{"tool_choice": {"any": {}}}"#),
        (thinking, r#"{"tool_choice": "required"}"#),
        (
            "{}",
            r#"{"thinking": {"type": "adaptive"}, "tool_choice": {"any": {}}}"#,
        ),
    ] {
        let untouched = harness.call(ANTHROPIC, params(non_default), params(optional), KEEP);
        assert!(untouched.infos().is_empty(), "{optional}");
        assert_eq!(
            untouched.mapped().get("tool_choice"),
            Some(&params(optional)["tool_choice"]),
            "{optional}"
        );
    }
}

fn none_tool_choice_dropped_or_refused(harness: &Harness) {
    for drop in [DROP_CALL, DROP_GLOBAL] {
        let dropped = harness.call(
            ANTHROPIC,
            params(r#"{"tool_choice": "none"}"#),
            params("{}"),
            drop,
        );
        assert_eq!(dropped.mapped(), params("{}"));
    }
    let refused = harness.call(
        ANTHROPIC,
        params(r#"{"tool_choice": "none"}"#),
        params("{}"),
        KEEP,
    );
    assert_eq!(refused.error().status_code(), Some(400));
}

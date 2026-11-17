use crate::converse_params::budget::update_thinking_budget;

use super::scenario::{ANTHROPIC, DROP_CALL, DROP_GLOBAL, KEEP, MODEL_COST, mapped};
use super::support::{Answers, Call, Fixture, params, run};

pub(super) fn thinking_budget_with_max_tokens() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let answers = Answers::new(&[]);
    let mut optional =
        params(r#"{"thinking": {"type": "disabled", "budget_tokens": 100}, "maxTokens": 5}"#);
    update_thinking_budget(&params("{}"), &mut optional, 1024).expect("the budget updates");
    assert_eq!(
        optional,
        params(r#"{"thinking": {"type": "disabled", "budget_tokens": 1024}, "maxTokens": 5}"#)
    );
    let enabled = params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#);
    let mut optional = enabled.clone();
    update_thinking_budget(&params("{}"), &mut optional, 1024).expect("the budget updates");
    assert_eq!(
        optional,
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "maxTokens": 6144}"#)
    );
    let mut optional =
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "maxTokens": 999}"#);
    update_thinking_budget(&params("{}"), &mut optional, 1024).expect("the budget updates");
    assert_eq!(
        optional,
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "maxTokens": 6144}"#)
    );
    for non_default in [
        r#"{"max_tokens": null}"#,
        r#"{"max_completion_tokens": null}"#,
        r#"{"max_tokens": 100}"#,
    ] {
        let mut optional = enabled.clone();
        update_thinking_budget(&params(non_default), &mut optional, 1024)
            .expect("the budget updates");
        assert_eq!(
            optional,
            params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#),
            "{non_default}"
        );
    }
    let mut optional = params(r#"{"thinking": {"type": "enabled", "budget_tokens": 1024.5}}"#);
    update_thinking_budget(&params("{}"), &mut optional, 1024).expect("the budget updates");
    assert_eq!(
        optional,
        params(
            r#"{"thinking": {"type": "enabled", "budget_tokens": 1024.5}, "maxTokens": 5120.5}"#
        )
    );
    let mut optional = params(r#"{"thinking": {"type": "enabled", "budget_tokens": 0}}"#);
    update_thinking_budget(&params("{}"), &mut optional, 1024).expect("the budget updates");
    assert_eq!(
        optional,
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 1024}, "maxTokens": 5120}"#)
    );
    for (budget, total) in [("true", 4097), ("false", 4096)] {
        let mut optional = params(&format!(
            r#"{{"thinking": {{"type": "enabled", "budget_tokens": {budget}}}}}"#
        ));
        update_thinking_budget(&params("{}"), &mut optional, 0).expect("the budget updates");
        assert_eq!(
            optional,
            params(&format!(
                r#"{{"thinking": {{"type": "enabled", "budget_tokens": {budget}}}, "maxTokens": {total}}}"#
            )),
            "{budget}"
        );
    }
    let mut optional = params(r#"{"thinking": {"type": "enabled", "budget_tokens": null}}"#);
    update_thinking_budget(&params("{}"), &mut optional, 1024).expect("the budget updates");
    assert_eq!(
        optional,
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": null}}"#)
    );
    let mut optional = params(r#"{"thinking": {"type": "enabled", "budget_tokens": "many"}}"#);
    let error = update_thinking_budget(&params("{}"), &mut optional, 1024)
        .expect_err("a text budget is not a number");
    assert_eq!(error.to_string(), "thinking.budget_tokens must be a number");
    for source in [
        r#"{"reasoning_effort": "high"}"#,
        r#"{"thinking": true}"#,
        "{}",
    ] {
        let mut optional = params(source);
        update_thinking_budget(&params("{}"), &mut optional, 1024).expect("the budget updates");
        assert_eq!(optional, params(source), "{source}");
    }
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            "openai.gpt-oss-120b-1:0",
            r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#
        )
        .mapped(),
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            "amazon.nova-2-lite-v1:0",
            r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#
        )
        .mapped(),
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"thinking": {"type": "enabled", "budget_tokens": 100}, "max_tokens": 9000}"#
        )
        .mapped(),
        params(r#"{"maxTokens": 9000, "thinking": {"type": "enabled", "budget_tokens": 1024}}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"thinking": {"type": "enabled", "budget_tokens": 100}}"#
        )
        .mapped(),
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 1024}, "maxTokens": 5120}"#)
    );
}

pub(super) fn adaptive_thinking_fallback() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let answers = Answers::new(&[]);
    for (max_tokens, budget) in [(3000, 2048), (1500, 1499)] {
        assert_eq!(
            mapped(
                &fixture,
                &mut session,
                &answers,
                ANTHROPIC,
                &format!(r#"{{"thinking": {{"type": "adaptive"}}, "max_tokens": {max_tokens}}}"#)
            )
            .mapped(),
            params(&format!(
                r#"{{"maxTokens": {max_tokens}, "thinking": {{"type": "enabled", "budget_tokens": {budget}}}}}"#
            )),
            "{max_tokens}"
        );
    }
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"thinking": {"type": "adaptive"}}"#
        )
        .mapped(),
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "maxTokens": 6144}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"thinking": {"type": "adaptive"}, "max_tokens": null}"#
        )
        .mapped(),
        params(r#"{"maxTokens": null, "thinking": {"type": "enabled", "budget_tokens": 2048}}"#)
    );
    let warned = mapped(
        &fixture,
        &mut session,
        &answers,
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
    let kept = run(
        &fixture,
        &mut session,
        &answers,
        Call {
            model: ANTHROPIC,
            non_default: params(r#"{"thinking": {"type": "adaptive"}, "max_tokens": 500}"#),
            optional: params(r#"{"thinking": {"type": "enabled", "budget_tokens": 5}}"#),
            drop: KEEP,
        },
    );
    assert_eq!(kept.warnings().len(), 1);
    assert_eq!(
        kept.mapped(),
        params(r#"{"maxTokens": 500, "thinking": {"type": "enabled", "budget_tokens": 1024}}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"thinking": {"type": "adaptive"}, "max_completion_tokens": 0, "max_tokens": 1200}"#
        )
        .mapped(),
        params(r#"{"maxTokens": 1200, "thinking": {"type": "enabled", "budget_tokens": 1199}}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"max_completion_tokens": 2000, "max_tokens": 1200, "thinking": {"type": "adaptive"}}"#
        )
        .mapped(),
        params(r#"{"maxTokens": 1200, "thinking": {"type": "enabled", "budget_tokens": 1999}}"#)
    );
    let error = mapped(
        &fixture,
        &mut session,
        &answers,
        ANTHROPIC,
        r#"{"thinking": {"type": "adaptive"}, "max_tokens": "900"}"#,
    )
    .error();
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(error.to_string(), "max_tokens must be a number");
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"thinking": {"type": "enabled", "budget_tokens": 5000}, "max_tokens": 6000}"#
        )
        .mapped(),
        params(r#"{"maxTokens": 6000, "thinking": {"type": "enabled", "budget_tokens": 5000}}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
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
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let answers = Answers::new(&[]);
    let downgraded = run(
        &fixture,
        &mut session,
        &answers,
        Call {
            model: ANTHROPIC,
            non_default: params(
                r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "max_tokens": 3000}"#,
            ),
            optional: params(r#"{"tool_choice": {"any": {}}}"#),
            drop: KEEP,
        },
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
    let specific = run(
        &fixture,
        &mut session,
        &answers,
        Call {
            model: ANTHROPIC,
            non_default: params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#),
            optional: params(r#"{"tool_choice": {"tool": {"name": "lookup"}}}"#),
            drop: KEEP,
        },
    );
    assert_eq!(
        specific.mapped(),
        params(
            r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "maxTokens": 6144, "tool_choice": {"auto": {}}}"#
        )
    );
    for (non_default, optional) in [
        (
            r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#,
            r#"{"tool_choice": {"auto": {}}}"#,
        ),
        ("{}", r#"{"tool_choice": {"any": {}}}"#),
        (
            r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#,
            r#"{"tool_choice": "required"}"#,
        ),
        (
            "{}",
            r#"{"thinking": {"type": "adaptive"}, "tool_choice": {"any": {}}}"#,
        ),
    ] {
        let outcome = run(
            &fixture,
            &mut session,
            &answers,
            Call {
                model: ANTHROPIC,
                non_default: params(non_default),
                optional: params(optional),
                drop: KEEP,
            },
        );
        assert!(outcome.infos().is_empty(), "{optional}");
        assert_eq!(
            outcome.mapped().get("tool_choice"),
            Some(&params(optional)["tool_choice"]),
            "{optional}"
        );
    }
    let gpt_oss = run(
        &fixture,
        &mut session,
        &answers,
        Call {
            model: "openai.gpt-oss-120b-1:0",
            non_default: params(r#"{"reasoning_effort": "high"}"#),
            optional: params(r#"{"tool_choice": {"any": {}}}"#),
            drop: KEEP,
        },
    );
    assert_eq!(
        gpt_oss.mapped(),
        params(r#"{"reasoning_effort": "high", "tool_choice": {"auto": {}}}"#)
    );
    for drop in [DROP_CALL, DROP_GLOBAL] {
        let dropped = run(
            &fixture,
            &mut session,
            &answers,
            Call {
                model: ANTHROPIC,
                non_default: params(r#"{"tool_choice": "none"}"#),
                optional: params("{}"),
                drop,
            },
        );
        assert_eq!(dropped.mapped(), params("{}"));
    }
    let refused = run(
        &fixture,
        &mut session,
        &answers,
        Call {
            model: ANTHROPIC,
            non_default: params(r#"{"tool_choice": "none"}"#),
            optional: params("{}"),
            drop: KEEP,
        },
    );
    assert_eq!(refused.error().status_code(), Some(400));
}

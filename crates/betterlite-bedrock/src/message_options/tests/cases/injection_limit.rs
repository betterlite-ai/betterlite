use betterlite_config::sections::anthropic_capability_catalog::{
    HIGH_THINKING_BUDGET, LOW_THINKING_BUDGET, MAX_THINKING_BUDGET, MEDIUM_THINKING_BUDGET,
    MINIMAL_THINKING_BUDGET, Settings, XHIGH_THINKING_BUDGET,
};
use tracing::Level;

use super::super::support::{Fixture, map};
use super::{MODEL_COST, PREFIX, clearing, ensure};

pub(super) fn clear_thinking_injection_limit() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let budgets = Settings::default();

    {
        let (result, request, messages) = ensure(
            &mut session,
            &budgets,
            "fixture-opus-4-5",
            clearing(r#""stream": true"#),
        );
        assert_eq!(result.ok(), Some(true));
        assert_eq!(
            request,
            clearing(r#""stream": true, "thinking": {"type": "enabled", "budget_tokens": 1024}"#)
        );
        assert_eq!(
            messages,
            vec![(
                Level::DEBUG,
                format!("{PREFIX} injected thinking with budget_tokens=1024")
            )]
        );
    }

    for (max_tokens, logged) in [
        ("1024", "1024"),
        ("1023", "1023"),
        ("0", "0"),
        ("-5", "-5"),
        ("true", "True"),
        ("false", "False"),
    ] {
        let (result, request, messages) = ensure(
            &mut session,
            &budgets,
            "fixture-opus-4-5",
            clearing(&format!(r#""max_tokens": {max_tokens}"#)),
        );
        assert_eq!(result.ok(), Some(false), "max_tokens {max_tokens}");
        assert_eq!(request, clearing(&format!(r#""max_tokens": {max_tokens}"#)));
        assert_eq!(
            messages,
            vec![(
                Level::WARN,
                format!(
                    "{PREFIX} max_tokens={logged} is not greater than minimum thinking budget (1024); cannot inject thinking safely"
                )
            )],
            "max_tokens {max_tokens}"
        );
    }

    for max_tokens in ["1025", "\"4096\"", "4096.5", "null"] {
        let (result, request, _messages) = ensure(
            &mut session,
            &budgets,
            "fixture-opus-4-5",
            clearing(&format!(r#""max_tokens": {max_tokens}"#)),
        );
        assert_eq!(result.ok(), Some(true), "max_tokens {max_tokens}");
        let expected = clearing(&format!(
            r#""max_tokens": {max_tokens}, "thinking": {{"type": "enabled", "budget_tokens": 1024}}"#
        ));
        assert_eq!(request, expected, "max_tokens {max_tokens}");
    }

    {
        // No Python int below i64::MIN: the guard is skipped where LiteLLM refuses.
        let pending = clearing(r#""max_tokens": -10000000000000000000000"#);
        let (result, request, _messages) =
            ensure(&mut session, &budgets, "fixture-opus-4-5", pending);
        assert_eq!(result.ok(), Some(true));
        assert_eq!(
            request,
            clearing(
                r#""max_tokens": -10000000000000000000000, "thinking": {"type": "enabled", "budget_tokens": 1024}"#
            )
        );
    }

    {
        let (result, request, messages) = ensure(
            &mut session,
            &budgets,
            "fixture-opus-4-5",
            clearing(r#""thinking": {}, "max_tokens": 1"#),
        );
        assert_eq!(result.ok(), Some(false));
        assert_eq!(request, clearing(r#""thinking": {}, "max_tokens": 1"#));
        assert_eq!(
            messages,
            vec![
                (
                    Level::DEBUG,
                    format!("{PREFIX} replacing thinking={{}} with minimal thinking config")
                ),
                (
                    Level::WARN,
                    format!(
                        "{PREFIX} max_tokens=1 is not greater than minimum thinking budget (1024); cannot inject thinking safely"
                    )
                ),
            ]
        );
    }

    {
        let (result, request, messages) = ensure(
            &mut session,
            &budgets,
            "fixture-opus-4-5",
            clearing(r#""thinking": {}, "max_tokens": 5000"#),
        );
        assert_eq!(result.ok(), Some(true));
        assert_eq!(
            request,
            clearing(
                r#""thinking": {"type": "enabled", "budget_tokens": 1024}, "max_tokens": 5000"#
            )
        );
        assert_eq!(
            messages,
            vec![
                (
                    Level::DEBUG,
                    format!("{PREFIX} replacing thinking={{}} with minimal thinking config")
                ),
                (
                    Level::DEBUG,
                    format!("{PREFIX} injected thinking with budget_tokens=1024")
                ),
            ]
        );
    }

    {
        let other = clearing(
            r#""thinking": {"type": "disabled", "budget_tokens": 5, "display": "summarized"}, "max_tokens": 5000"#,
        );
        let (result, request, messages) = ensure(&mut session, &budgets, "fixture-opus-4-5", other);
        assert_eq!(result.ok(), Some(true));
        assert_eq!(
            request,
            clearing(
                r#""thinking": {"type": "enabled", "budget_tokens": 1024}, "max_tokens": 5000"#
            )
        );
        assert_eq!(
            messages,
            vec![
                (
                    Level::DEBUG,
                    format!(
                        "{PREFIX} replacing thinking={{\"type\":\"disabled\",\"budget_tokens\":5,\"display\":\"summarized\"}} with minimal thinking config"
                    )
                ),
                (
                    Level::DEBUG,
                    format!("{PREFIX} injected thinking with budget_tokens=1024")
                ),
            ]
        );
    }

    {
        let (result, request, messages) = ensure(
            &mut session,
            &budgets,
            "fixture-opus-4-5",
            clearing(r#""thinking": "adaptive", "max_tokens": 5000"#),
        );
        assert_eq!(result.ok(), Some(true));
        assert_eq!(
            request,
            clearing(
                r#""thinking": {"type": "enabled", "budget_tokens": 1024}, "max_tokens": 5000"#
            )
        );
        assert_eq!(
            messages,
            vec![(
                Level::DEBUG,
                format!("{PREFIX} injected thinking with budget_tokens=1024")
            )]
        );
    }

    let tight = Settings {
        minimal: MINIMAL_THINKING_BUDGET,
        low: LOW_THINKING_BUDGET,
        medium: MEDIUM_THINKING_BUDGET,
        high: HIGH_THINKING_BUDGET,
        xhigh: XHIGH_THINKING_BUDGET,
        max: MAX_THINKING_BUDGET,
        bedrock_min: 5,
    };
    {
        let (result, _request, messages) = ensure(
            &mut session,
            &tight,
            "fixture-opus-4-5",
            clearing(r#""max_tokens": 5"#),
        );
        assert_eq!(result.ok(), Some(false));
        assert_eq!(
            messages,
            vec![(
                Level::WARN,
                format!(
                    "{PREFIX} max_tokens=5 is not greater than minimum thinking budget (5); cannot inject thinking safely"
                )
            )]
        );
    }
    {
        let (result, request, messages) = ensure(
            &mut session,
            &tight,
            "fixture-opus-4-5",
            clearing(r#""max_tokens": 6"#),
        );
        assert_eq!(result.ok(), Some(true));
        assert_eq!(
            request,
            clearing(r#""max_tokens": 6, "thinking": {"type": "enabled", "budget_tokens": 5}"#)
        );
        assert_eq!(
            messages,
            vec![(
                Level::DEBUG,
                format!("{PREFIX} injected thinking with budget_tokens=5")
            )]
        );
    }

    {
        let (result, request, messages) = ensure(
            &mut session,
            &budgets,
            "fixture-adaptive",
            clearing(r#""max_tokens": 1"#),
        );
        assert_eq!(result.ok(), Some(false));
        assert_eq!(request, clearing(r#""max_tokens": 1"#));
        assert_eq!(
            messages,
            vec![(
                Level::WARN,
                format!(
                    "{PREFIX} max_tokens=1 is not greater than minimum thinking budget (1024); cannot inject thinking safely"
                )
            )]
        );
    }

    {
        let (result, request, messages) = ensure(
            &mut session,
            &budgets,
            "fixture-adaptive",
            clearing(r#""max_tokens": 5000"#),
        );
        assert_eq!(result.ok(), Some(true));
        assert_eq!(
            request,
            clearing(
                r#""max_tokens": 5000, "output_config": {"effort": "low"}, "thinking": {"type": "adaptive"}"#
            )
        );
        assert_eq!(
            messages,
            vec![(
                Level::DEBUG,
                format!(
                    "{PREFIX} injected adaptive thinking with effort=low for model=fixture-adaptive"
                )
            )]
        );
    }

    {
        let no_edit = map(
            r#"{"context_management": {"edits": [{"type": "other"}]}, "thinking": {}, "max_tokens": 1}"#,
        );
        let (result, request, messages) =
            ensure(&mut session, &budgets, "fixture-opus-4-5", no_edit.clone());
        assert_eq!(result.ok(), Some(false));
        assert_eq!(request, no_edit);
        assert!(messages.is_empty());
    }

    {
        let unsupported = clearing(r#""thinking": {}"#);
        let (result, request, messages) = ensure(
            &mut session,
            &budgets,
            "fixture-legacy",
            unsupported.clone(),
        );
        assert_eq!(result.ok(), Some(false));
        assert_eq!(request, unsupported);
        assert!(messages.is_empty());
    }
}

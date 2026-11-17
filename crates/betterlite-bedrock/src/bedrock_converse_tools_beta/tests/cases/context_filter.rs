use super::super::support::{Session, list, params};
use super::{ADAPTIVE_MODEL, FOREIGN_MODEL, prepare};

pub(super) fn removed_foreign(session: &mut Session<'_>) {
    let foreign = prepare(
        session,
        FOREIGN_MODEL,
        "[]",
        None,
        r#"{"context_management": {"edits": [{"type": "clear_thinking_20251015"}]}}"#,
    );
    assert_eq!(
        foreign.params,
        params(r#"{"context_management": {"edits": [{"type": "clear_thinking_20251015"}]}}"#)
    );
    assert!(foreign.betas.is_empty());

    let text = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        None,
        r#"{"context_management": "compact"}"#,
    );
    assert_eq!(text.params, params("{}"));

    let null = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        None,
        r#"{"context_management": null}"#,
    );
    assert_eq!(null.params, params("{}"));
}

pub(super) fn removed_shapes(session: &mut Session<'_>) {
    let edits = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        None,
        r#"{"context_management": {"edits": {"type": "compact_20260112"}}}"#,
    );
    assert_eq!(edits.params, params("{}"));

    let missing = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        None,
        r#"{"context_management": {"keep": true}}"#,
    );
    assert_eq!(missing.params, params("{}"));

    let survivors = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        None,
        r#"{"context_management": {"edits": [{"type": "clear_thinking_20251015"}, "compact_20260112"]}}"#,
    );
    assert_eq!(survivors.params, params("{}"));
    assert!(survivors.betas.is_empty());
}

pub(super) fn retained_edits(session: &mut Session<'_>) {
    let kept = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        None,
        r#"{"context_management": {"edits": [{"type": "clear_thinking_20251015"}, {"type": "compact_20260112", "keep": "all"}, {"type": "compact_20260112"}], "extra": {"scope": "all"}}, "max_tokens": 9}"#,
    );
    assert_eq!(
        kept.params,
        params(
            r#"{"context_management": {"edits": [{"type": "compact_20260112", "keep": "all"}, {"type": "compact_20260112"}], "extra": {"scope": "all"}}, "max_tokens": 9, "anthropic_beta": ["compact-2026-01-12"]}"#
        )
    );
    assert_eq!(kept.betas, list(r#"["compact-2026-01-12"]"#));
}

pub(super) fn retained_deduplicated(session: &mut Session<'_>) {
    let deduplicated = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        Some(r#"{"anthropic-beta": "compact-2026-01-12"}"#),
        r#"{"context_management": {"edits": [{"type": "compact_20260112"}]}}"#,
    );
    assert_eq!(deduplicated.betas, list(r#"["compact-2026-01-12"]"#));
}

pub(super) fn retained_shapes(session: &mut Session<'_>) {
    let non_dict = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        None,
        r#"{"context_management": {"edits": ["compact_20260112"]}}"#,
    );
    assert_eq!(non_dict.params, params("{}"));

    let other_fields = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        None,
        r#"{"context_management": {"edits": [{"type": "compact_20260112"}]}}"#,
    );
    assert_eq!(
        other_fields.params,
        params(
            r#"{"context_management": {"edits": [{"type": "compact_20260112"}]}, "anthropic_beta": ["compact-2026-01-12"]}"#
        )
    );
    assert_eq!(other_fields.betas, list(r#"["compact-2026-01-12"]"#));
}

use super::super::support::{Session, list, params};
use super::{ADAPTIVE_MODEL, FOREIGN_MODEL, prepare};

const NON_DICT_EDITS: &str = r#"{"context_management": {"edits": ["compact_20260112"]}}"#;
const COMPACT_EDITS: &str = r#"{"context_management": {"edits": [{"type": "compact_20260112"}]}}"#;
const COMPACT_PARAMS: &str = r#"{"context_management": {"edits": [{"type": "compact_20260112"}]}, "anthropic_beta": ["compact-2026-01-12"]}"#;
const COMPACT_BETAS: &str = r#"["compact-2026-01-12"]"#;

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
    let dropped = [
        r#"{"context_management": {"edits": {"type": "compact_20260112"}}}"#,
        r#"{"context_management": {"keep": true}}"#,
        r#"{"context_management": {"edits": [{"type": "clear_thinking_20251015"}, "compact_20260112"]}}"#,
    ];
    for param in dropped {
        let removed = prepare(session, ADAPTIVE_MODEL, "[]", None, param);
        assert_eq!(removed.params, params("{}"));
        assert!(removed.betas.is_empty());
    }
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
    assert_eq!(kept.betas, list(COMPACT_BETAS));

    let deduplicated = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        Some(r#"{"anthropic-beta": "compact-2026-01-12"}"#),
        COMPACT_EDITS,
    );
    assert_eq!(deduplicated.betas, list(COMPACT_BETAS));
}

pub(super) fn retained_non_dict_edits(session: &mut Session<'_>) {
    let non_dict = prepare(session, ADAPTIVE_MODEL, "[]", None, NON_DICT_EDITS);
    assert_eq!(non_dict.params, params("{}"));
}

pub(super) fn retained_compact_fields(session: &mut Session<'_>) {
    let compact = prepare(session, ADAPTIVE_MODEL, "[]", None, COMPACT_EDITS);
    assert_eq!(compact.params, params(COMPACT_PARAMS));
    assert_eq!(compact.betas, list(COMPACT_BETAS));
}

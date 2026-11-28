use betterlite_models::decisions::Json;

use crate::api::messages_chat_prepare::prediction::predict_context_edit_execution as predict_edits;

use super::support::parse;

fn prediction(spec: Option<&Json>, drops: Option<&Json>) -> (bool, bool) {
    let predicted = predict_edits(spec, drops);
    (predicted.compact, predicted.non_compact)
}

#[test]
fn predict_context_edit_execution() {
    let compact = parse(
        r#"{"edits":[{"type":"compact_20260112","trigger":{"type":"input_tokens","value":60000}}]}"#,
    );

    assert_disabled_specs();
    assert_drop_parameter(&compact);
    assert_compact_forms(&compact);
    assert_non_compact_edits();
    assert_untyped_edits();
}

// absent, null and empty specs disable the polyfill, so neither kind of edit is predicted
fn assert_disabled_specs() {
    assert_eq!(prediction(None, None), (false, false));
    assert_eq!(prediction(Some(&parse("null")), None), (false, false));
    assert_eq!(prediction(Some(&parse("{}")), None), (false, false));
    assert_eq!(prediction(Some(&parse("[]")), None), (false, false));
    assert_eq!(
        prediction(Some(&parse(r#"{"edits":[]}"#)), None),
        (false, false)
    );
}

// only a list naming context_management disables; a non-list drop parameter never does
fn assert_drop_parameter(compact: &Json) {
    assert_eq!(
        prediction(Some(compact), Some(&parse(r#"["context_management"]"#))),
        (false, false)
    );
    assert_eq!(
        prediction(
            Some(compact),
            Some(&parse(r#"["temperature","context_management"]"#))
        ),
        (false, false)
    );
    assert_eq!(
        prediction(Some(compact), Some(&parse(r#""context_management""#))),
        (true, false)
    );
    assert_eq!(
        prediction(
            Some(compact),
            Some(&parse(r#"{"context_management":true}"#))
        ),
        (true, false)
    );
    assert_eq!(prediction(Some(compact), Some(&parse("7"))), (true, false));
    assert_eq!(
        prediction(Some(compact), Some(&parse(r#"["temperature"]"#))),
        (true, false)
    );
    assert_eq!(
        prediction(Some(compact), Some(&parse(r#"[["context_management"]]"#))),
        (true, false)
    );
}

// a native compact edit, and the OpenAI list form the dispatcher normalizes into it
fn assert_compact_forms(compact: &Json) {
    assert_eq!(prediction(Some(compact), None), (true, false));
    assert_eq!(
        prediction(
            Some(&parse(
                r#"[{"type":"compaction","compact_threshold":60000}]"#
            )),
            None
        ),
        (true, false)
    );
    assert_eq!(
        prediction(Some(&parse(r#"[{"type":"compaction"}]"#)), None),
        (true, false)
    );
    assert_eq!(
        prediction(
            Some(&parse(
                r#"[{"type":"compaction","compact_threshold":60000.9}]"#
            )),
            None
        ),
        (true, false)
    );
}

// any other named edit is non-compact, whether the dispatcher knows it or not
fn assert_non_compact_edits() {
    assert_eq!(
        prediction(
            Some(&parse(
                r#"{"edits":[{"type":"clear_tool_uses_20250919","trigger":{"type":"input_tokens","value":50000}}]}"#
            )),
            None
        ),
        (false, true)
    );
    assert_eq!(
        prediction(Some(&parse(r#"{"edits":[{"type":"mystery_edit"}]}"#)), None),
        (false, true)
    );
    assert_eq!(
        prediction(
            Some(&parse(
                r#"{"edits":[{"type":"clear_tool_uses_20250919"},{"type":"compact_20260112"}]}"#
            )),
            None
        ),
        (true, true)
    );
    assert_eq!(
        prediction(
            Some(&parse(
                r#"{"edits":[{"type":"mystery_edit"},{"type":"compact_20260112"}]}"#
            )),
            None
        ),
        (true, true)
    );
}

// an edit with no string type predicts nothing, and an OpenAI list without compaction maps to no edits
fn assert_untyped_edits() {
    assert_eq!(
        prediction(Some(&parse(r#"{"edits":[{}]}"#)), None),
        (false, false)
    );
    assert_eq!(
        prediction(Some(&parse(r#"{"edits":[{"type":7}]}"#)), None),
        (false, false)
    );
    assert_eq!(
        prediction(
            Some(&parse(r#"[{"type":"clear_tool_uses_20250919"}]"#)),
            None
        ),
        (false, false)
    );
    assert_eq!(
        prediction(Some(&parse(r#"[{"type":"compaction"},7]"#)), None),
        (true, false)
    );
    assert_eq!(prediction(Some(&parse("[7]")), None), (false, false));
    // a spec the normalizer refuses is swallowed to no edits, so the wrapper falls back to history
    assert_eq!(
        prediction(
            Some(&parse(
                r#"[{"type":"compaction","compact_threshold":1e400}]"#
            )),
            None
        ),
        (false, false)
    );
}

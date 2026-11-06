use betterlite_guardrails::vigil_guard::vigil_guard_sanitize::resolve_sanitized_text;

use crate::support::analysis;

#[test]
fn sanitized_text_wins_over_output_text() {
    let analysis = analysis(r#"{"sanitizedText":"clean","outputText":"other"}"#);
    assert_eq!(resolve_sanitized_text("original", &analysis), "clean");
}

#[test]
fn output_text_is_the_fallback() {
    let analysis = analysis(r#"{"outputText":"other"}"#);
    assert_eq!(resolve_sanitized_text("original", &analysis), "other");
}

#[test]
fn the_original_is_kept_without_either_field() {
    let analysis = analysis("{}");
    assert_eq!(resolve_sanitized_text("original", &analysis), "original");
}

#[test]
fn an_empty_sanitized_text_is_used_as_given() {
    let analysis = analysis(r#"{"sanitizedText":"","outputText":"other"}"#);
    assert_eq!(resolve_sanitized_text("original", &analysis), "");
}

#[test]
fn a_non_string_sanitized_text_falls_through() {
    let analysis = analysis(r#"{"sanitizedText":42,"outputText":"other"}"#);
    assert_eq!(resolve_sanitized_text("original", &analysis), "other");
}

#[test]
fn a_non_string_output_text_keeps_the_original() {
    let analysis = analysis(r#"{"outputText":42}"#);
    assert_eq!(resolve_sanitized_text("original", &analysis), "original");
}

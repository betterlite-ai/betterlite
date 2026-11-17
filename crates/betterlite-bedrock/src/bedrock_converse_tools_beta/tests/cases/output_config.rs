use super::super::support::{Session, list, params};
use super::{ADAPTIVE_MODEL, PLAIN_MODEL, prepare};

pub(super) fn preserved(session: &mut Session<'_>) {
    let formatted = r#"{"output_config": {"effort": "high", "format": {"type": "json_schema", "schema": {"type": "object"}}}}"#;
    let adaptive = prepare(session, ADAPTIVE_MODEL, "[]", None, formatted);
    assert!(adaptive.betas.is_empty());
    assert_eq!(adaptive.params, params(formatted));

    let plain = prepare(
        session,
        PLAIN_MODEL,
        "[]",
        None,
        r#"{"output_config": {"effort": "high", "format": {"type": "json_schema"}}}"#,
    );
    assert_eq!(plain.betas, list(r#"["effort-2025-11-24"]"#));
    assert_eq!(
        plain.params,
        params(
            r#"{"output_config": {"effort": "high", "format": {"type": "json_schema"}}, "anthropic_beta": ["effort-2025-11-24"]}"#
        )
    );

    let absent = prepare(session, ADAPTIVE_MODEL, "[]", None, r#"{"max_tokens": 8}"#);
    assert_eq!(absent.params, params(r#"{"max_tokens": 8}"#));

    let null = prepare(
        session,
        PLAIN_MODEL,
        "[]",
        None,
        r#"{"output_config": null}"#,
    );
    assert!(null.betas.is_empty());
    assert_eq!(null.params, params(r#"{"output_config": null}"#));
}

pub(super) fn unrelated_params(session: &mut Session<'_>) {
    let tools = r#"[{"type": "function", "function": {"name": "lookup", "parameters": {"type": "object", "properties": {}}}}]"#;
    let source = r#"{"tool_choice": {"type": "tool_choice", "function": {"name": "lookup"}}, "parallel_tool_calls": false, "toolConfig": {"tools": []}}"#;
    let processed = prepare(session, ADAPTIVE_MODEL, tools, None, source);
    assert_eq!(processed.params, params(source));
    assert_eq!(processed.tools.len(), 1);

    let plain = prepare(
        session,
        PLAIN_MODEL,
        tools,
        None,
        r#"{"output_config": {"effort": "low"}, "tool_choice": "auto"}"#,
    );
    assert_eq!(
        plain.params,
        params(
            r#"{"output_config": {"effort": "low"}, "tool_choice": "auto", "anthropic_beta": ["effort-2025-11-24"]}"#
        )
    );
}

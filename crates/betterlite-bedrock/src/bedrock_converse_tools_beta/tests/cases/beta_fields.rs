use super::super::support::{Request, Session, list, params};
use super::{ADAPTIVE_MODEL, FOREIGN_MODEL, PLAIN_MODEL, prepare};

const EAGER_TOOL: &str =
    r#"[{"type": "function", "function": {"name": "f", "eager_input_streaming": true}}]"#;

pub(super) fn client_headers(session: &mut Session<'_>) {
    let absent = prepare(session, ADAPTIVE_MODEL, "[]", None, "{}");
    assert!(absent.betas.is_empty());
    assert_eq!(absent.params, params("{}"));

    let empty = prepare(session, ADAPTIVE_MODEL, "[]", Some("{}"), "{}");
    assert!(empty.betas.is_empty());

    let listed = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        Some(r#"{"anthropic-beta": ["beta-a", "beta-b", "beta-a"]}"#),
        "{}",
    );
    assert_eq!(listed.betas, list(r#"["beta-a", "beta-b", "beta-a"]"#));
    assert_eq!(
        listed.params,
        params(r#"{"anthropic_beta": ["beta-a", "beta-b", "beta-a"]}"#)
    );

    let text = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        Some(r#"{"anthropic-beta": "beta-a, beta-b"}"#),
        "{}",
    );
    assert_eq!(text.betas, list(r#"["beta-a", "beta-b"]"#));

    let array = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        Some(r#"{"anthropic-beta": "[\"beta-a\", 7]"}"#),
        "{}",
    );
    assert_eq!(array.betas, list(r#"["beta-a", "7"]"#));
}

pub(super) fn eager_added(session: &mut Session<'_>) {
    let added = prepare(session, ADAPTIVE_MODEL, EAGER_TOOL, None, "{}");
    assert_eq!(
        added.betas,
        list(r#"["fine-grained-tool-streaming-2025-05-14"]"#)
    );
    assert_eq!(
        added.params,
        params(r#"{"anthropic_beta": ["fine-grained-tool-streaming-2025-05-14"]}"#)
    );
}

pub(super) fn eager_deduplicated(session: &mut Session<'_>) {
    let deduplicated = prepare(
        session,
        ADAPTIVE_MODEL,
        EAGER_TOOL,
        Some(r#"{"anthropic-beta": "fine-grained-tool-streaming-2025-05-14,beta-x"}"#),
        "{}",
    );
    assert_eq!(
        deduplicated.betas,
        list(r#"["fine-grained-tool-streaming-2025-05-14", "beta-x"]"#)
    );

    let foreign = prepare(session, FOREIGN_MODEL, EAGER_TOOL, None, "{}");
    assert!(foreign.betas.is_empty());
    assert_eq!(foreign.params, params("{}"));

    let prefixed = prepare(
        session,
        ADAPTIVE_MODEL,
        r#"[{"systemTool": null, "eager_input_streaming": true}]"#,
        None,
        "{}",
    );
    assert!(prefixed.betas.is_empty());
}

pub(super) fn eager_malformed(session: &mut Session<'_>) {
    let broken = session.process(&Request::new(
        ADAPTIVE_MODEL,
        r#"[{"type": "function", "function": {"name": "f", "eager_input_streaming": "yes"}}]"#,
        None,
        "{}",
    ));
    match broken {
        Err(error) => assert_eq!(error.status_code(), Some(400)),
        Ok(_) => panic!("a malformed eager flag must be refused"),
    }

    let foreign = prepare(
        session,
        FOREIGN_MODEL,
        r#"[{"type": "function", "function": {"name": "f", "eager_input_streaming": "yes"}}]"#,
        None,
        "{}",
    );
    assert!(foreign.betas.is_empty());
}

pub(super) fn effort_values(session: &mut Session<'_>) {
    let high = r#"{"output_config": {"effort": "high"}}"#;
    let plain = prepare(session, PLAIN_MODEL, "[]", None, high);
    assert_eq!(plain.betas, list(r#"["effort-2025-11-24"]"#));
    assert_eq!(
        plain.params,
        params(r#"{"output_config": {"effort": "high"}, "anthropic_beta": ["effort-2025-11-24"]}"#)
    );

    let adaptive = prepare(session, ADAPTIVE_MODEL, "[]", None, high);
    assert!(adaptive.betas.is_empty());
    assert_eq!(adaptive.params, params(high));

    let prefixed = prepare(session, "bedrock/anthropic.fixture-plain", "[]", None, high);
    assert_eq!(prefixed.betas, plain.betas);
    assert_eq!(prefixed.params, plain.params);

    let duplicate = prepare(
        session,
        PLAIN_MODEL,
        "[]",
        Some(r#"{"anthropic-beta": "effort-2025-11-24"}"#),
        high,
    );
    assert_eq!(duplicate.betas, list(r#"["effort-2025-11-24"]"#));

    let unrecognized = prepare(
        session,
        PLAIN_MODEL,
        "[]",
        None,
        r#"{"output_config": {"effort": "banana"}}"#,
    );
    assert_eq!(unrecognized.betas, list(r#"["effort-2025-11-24"]"#));
}

pub(super) fn effort_without_effect(session: &mut Session<'_>) {
    let null = prepare(
        session,
        PLAIN_MODEL,
        "[]",
        None,
        r#"{"output_config": {"effort": null}}"#,
    );
    assert!(null.betas.is_empty());

    let text = prepare(
        session,
        PLAIN_MODEL,
        "[]",
        None,
        r#"{"output_config": "high"}"#,
    );
    assert!(text.betas.is_empty());

    let foreign = prepare(
        session,
        FOREIGN_MODEL,
        "[]",
        None,
        r#"{"output_config": {"effort": "high"}}"#,
    );
    assert!(foreign.betas.is_empty());
    assert_eq!(
        foreign.params,
        params(r#"{"output_config": {"effort": "high"}}"#)
    );
}

pub(super) fn parameter_writing(session: &mut Session<'_>) {
    let foreign = prepare(
        session,
        FOREIGN_MODEL,
        "[]",
        Some(r#"{"anthropic-beta": ["beta-a"]}"#),
        r#"{"anthropic_beta": ["kept"]}"#,
    );
    assert_eq!(foreign.betas, list(r#"["beta-a"]"#));
    assert_eq!(foreign.params, params(r#"{"anthropic_beta": ["kept"]}"#));

    let empty = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        None,
        r#"{"anthropic_beta": ["kept"]}"#,
    );
    assert!(empty.betas.is_empty());
    assert_eq!(empty.params, params(r#"{"anthropic_beta": ["kept"]}"#));

    let replaced = prepare(
        session,
        ADAPTIVE_MODEL,
        "[]",
        Some(r#"{"anthropic-beta": ["beta-a"]}"#),
        r#"{"anthropic_beta": ["kept"]}"#,
    );
    assert_eq!(replaced.params, params(r#"{"anthropic_beta": ["beta-a"]}"#));
}

use betterlite_openai::reasoning_models::DropParams;
use tracing::Level;

use crate::mantle_responses::error::MantleResponsesError;
use crate::mantle_responses::options::{drop_unsupported_summary, drop_unsupported_tier};

use super::support;

const MODEL: &str = "openai.gpt-oss-120b";

const TIER_WARNING: &str = "Bedrock Mantle Responses API: dropping unsupported service_tier 'flex' (supported: ['auto', 'default']).";

const DROPPING: DropParams = DropParams {
    global: false,
    request: true,
};

const REJECTING: DropParams = DropParams {
    global: false,
    request: false,
};

const GLOBAL: DropParams = DropParams {
    global: true,
    request: false,
};

const SUMMARY_WARNING: &str = "Bedrock Mantle Responses API: dropping unsupported reasoning.summary 'concise' (supported: ['auto']).";

pub(super) fn unsupported_params_warn_drop() {
    dropped_cases();
    error_cases();
    standard_path_cases();
    repr_cases();
}

const DROPPED_CASES: [(&str, &str, &str); 6] = [
    (
        r#"{"service_tier": "flex", "temperature": 0.5}"#,
        r#"{"temperature": 0.5}"#,
        TIER_WARNING,
    ),
    (
        r#"{"service_tier": 3}"#,
        "{}",
        "Bedrock Mantle Responses API: dropping unsupported service_tier 3 (supported: ['auto', 'default']).",
    ),
    (
        r#"{"service_tier": true}"#,
        "{}",
        "Bedrock Mantle Responses API: dropping unsupported service_tier True (supported: ['auto', 'default']).",
    ),
    (
        r#"{"reasoning": {"summary": "concise", "effort": "high"}}"#,
        r#"{"reasoning": {"effort": "high"}}"#,
        SUMMARY_WARNING,
    ),
    (
        r#"{"reasoning": {"summary": "concise"}}"#,
        "{}",
        SUMMARY_WARNING,
    ),
    (
        r#"{"reasoning": {"summary": 5}, "temperature": 0.5}"#,
        r#"{"temperature": 0.5}"#,
        "Bedrock Mantle Responses API: dropping unsupported reasoning.summary 5 (supported: ['auto']).",
    ),
];

fn dropped_cases() {
    let (messages, _guard) = support::capture();
    for (index, (source, expected, warning)) in DROPPED_CASES.into_iter().enumerate() {
        let params = support::object(source);
        let original = params.clone();
        let tiered = drop_unsupported_tier(&params, DROPPING).expect("the tier drops");
        let dropped = drop_unsupported_summary(&support::config(true), &tiered, MODEL, DROPPING)
            .expect("the summary drops");
        assert_eq!(dropped, support::object(expected), "params {source}");
        assert_eq!(params, original, "params {source}");
        let warned = messages.at(Level::WARN);
        assert_eq!(warned.len(), index.saturating_add(1), "params {source}");
        assert_eq!(warned.last().map(String::as_str), Some(warning));
    }
}

fn error_cases() {
    let (messages, _guard) = support::capture();
    let params = support::object(r#"{"service_tier": "flex"}"#);
    let error = drop_unsupported_tier(&params, REJECTING).expect_err("the tier is rejected");
    let MantleResponsesError::UnsupportedParams { message } = &error else {
        panic!("expected an unsupported params error");
    };
    assert!(message.starts_with("bedrock_mantle does not support service_tier='flex';"));
    let global = drop_unsupported_tier(&params, GLOBAL).expect("the global switch drops it");
    assert_eq!(global, support::object("{}"));
    assert_eq!(messages.at(Level::WARN), vec![TIER_WARNING.to_owned()]);
    summary_error_case();
}

fn summary_error_case() {
    let reasoning = support::object(r#"{"reasoning": {"summary": "concise"}}"#);
    let error = drop_unsupported_summary(&support::config(true), &reasoning, MODEL, REJECTING)
        .expect_err("the summary is rejected");
    let MantleResponsesError::UnsupportedParams { message } = &error else {
        panic!("expected an unsupported params error");
    };
    assert!(message.starts_with(
        "bedrock_mantle does not support reasoning.summary='concise' for 'openai.gpt-oss-120b';"
    ));
}

fn standard_path_cases() {
    let (messages, _guard) = support::capture();
    let params = support::object(r#"{"reasoning": {"summary": "concise"}}"#);
    let kept = drop_unsupported_summary(&support::config(false), &params, MODEL, REJECTING)
        .expect("the standard path keeps every summary");
    assert_eq!(kept, params);
    assert!(messages.at(Level::WARN).is_empty());
}

fn repr_cases() {
    let (messages, _guard) = support::capture();
    let cases = [
        (r#"{"service_tier": "it's"}"#, r#""it's""#),
        (r#"{"service_tier": "a'b\"c"}"#, r#"'a\'b"c'"#),
        (r#"{"service_tier": "x\\y\n\t\r"}"#, r"'x\\y\n\t\r'"),
    ];
    for (source, shown) in cases {
        let dropped = drop_unsupported_tier(&support::object(source), DROPPING);
        assert_eq!(dropped.expect("the tier drops"), support::object("{}"));
        let warned = messages.at(Level::WARN).pop().expect("a warning");
        assert_eq!(
            warned,
            format!(
                "Bedrock Mantle Responses API: dropping unsupported service_tier {shown} (supported: ['auto', 'default'])."
            )
        );
    }
}

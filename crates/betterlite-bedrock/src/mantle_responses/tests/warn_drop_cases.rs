use betterlite_openai::reasoning_models::DropParams;
use betterlite_prompts::arguments::json::Object;
use tracing::Level;

use crate::mantle_responses::error::MantleResponsesError;
use crate::mantle_responses::options::{drop_unsupported_summary, drop_unsupported_tier};

use super::support;

const MODEL: &str = "openai.gpt-oss-120b";

const TIER_WARNING: &str = "Bedrock Mantle Responses API: dropping unsupported service_tier 'flex' (supported: ['auto', 'default']).";

const SUMMARY_WARNING: &str = "Bedrock Mantle Responses API: dropping unsupported reasoning.summary 'concise' (supported: ['auto']).";

pub(super) fn unsupported_params_warn_drop() {
    tier_cases();
    summary_cases();
    error_cases();
    standard_path_cases();
    repr_cases();
}

fn drop_tier(params: &Object) -> Object {
    drop_unsupported_tier(
        params,
        DropParams {
            global: false,
            request: true,
        },
    )
    .expect("the tier drops")
}

fn drop_summary(params: &Object) -> Object {
    drop_unsupported_summary(
        &support::config(true),
        params,
        MODEL,
        DropParams {
            global: false,
            request: true,
        },
    )
    .expect("the summary drops")
}

fn tier_cases() {
    let (messages, _guard) = support::capture();
    let params = support::object(r#"{"service_tier": "flex", "temperature": 0.5}"#);
    let original = params.clone();
    let dropped = drop_tier(&params);
    assert_eq!(dropped, support::object(r#"{"temperature": 0.5}"#));
    assert_eq!(params, original);
    assert_eq!(messages.at(Level::WARN), vec![TIER_WARNING.to_owned()]);
    let numeric = support::object(r#"{"service_tier": 3}"#);
    assert_eq!(drop_tier(&numeric), support::object("{}"));
    let boolean = support::object(r#"{"service_tier": true}"#);
    assert_eq!(drop_tier(&boolean), support::object("{}"));
    assert_eq!(
        messages.at(Level::WARN),
        vec![
            TIER_WARNING.to_owned(),
            "Bedrock Mantle Responses API: dropping unsupported service_tier 3 (supported: ['auto', 'default']).".to_owned(),
            "Bedrock Mantle Responses API: dropping unsupported service_tier True (supported: ['auto', 'default']).".to_owned(),
        ]
    );
}

fn summary_cases() {
    let (messages, _guard) = support::capture();
    let params = support::object(r#"{"reasoning": {"summary": "concise", "effort": "high"}}"#);
    let original = params.clone();
    let dropped = drop_summary(&params);
    assert_eq!(
        dropped,
        support::object(r#"{"reasoning": {"effort": "high"}}"#)
    );
    assert_eq!(params, original);
    assert_eq!(messages.at(Level::WARN), vec![SUMMARY_WARNING.to_owned()]);
    let only = support::object(r#"{"reasoning": {"summary": "concise"}}"#);
    assert_eq!(drop_summary(&only), support::object("{}"));
    let numeric = support::object(r#"{"reasoning": {"summary": 5}, "temperature": 0.5}"#);
    assert_eq!(
        drop_summary(&numeric),
        support::object(r#"{"temperature": 0.5}"#)
    );
    assert_eq!(
        messages.at(Level::WARN),
        vec![
            SUMMARY_WARNING.to_owned(),
            SUMMARY_WARNING.to_owned(),
            "Bedrock Mantle Responses API: dropping unsupported reasoning.summary 5 (supported: ['auto']).".to_owned(),
        ]
    );
}

fn error_cases() {
    let (messages, _guard) = support::capture();
    let params = support::object(r#"{"service_tier": "flex"}"#);
    let error = drop_unsupported_tier(
        &params,
        DropParams {
            global: false,
            request: false,
        },
    )
    .expect_err("the tier is rejected");
    let MantleResponsesError::UnsupportedParams { message } = &error else {
        panic!("expected an unsupported params error");
    };
    assert!(message.starts_with("bedrock_mantle does not support service_tier='flex';"));
    let global = drop_unsupported_tier(
        &params,
        DropParams {
            global: true,
            request: false,
        },
    )
    .expect("the global switch drops it");
    assert_eq!(global, support::object("{}"));
    let reasoning = support::object(r#"{"reasoning": {"summary": "concise"}}"#);
    let summary = drop_unsupported_summary(
        &support::config(true),
        &reasoning,
        MODEL,
        DropParams {
            global: false,
            request: false,
        },
    )
    .expect_err("the summary is rejected");
    let MantleResponsesError::UnsupportedParams { message } = &summary else {
        panic!("expected an unsupported params error");
    };
    assert!(message.starts_with(
        "bedrock_mantle does not support reasoning.summary='concise' for 'openai.gpt-oss-120b';"
    ));
    assert_eq!(messages.at(Level::WARN), vec![TIER_WARNING.to_owned()]);
}

fn standard_path_cases() {
    let (messages, _guard) = support::capture();
    let params = support::object(r#"{"reasoning": {"summary": "concise"}}"#);
    let kept = drop_unsupported_summary(
        &support::config(false),
        &params,
        MODEL,
        DropParams {
            global: false,
            request: false,
        },
    )
    .expect("the standard path keeps every summary");
    assert_eq!(kept, params);
    assert!(messages.at(Level::WARN).is_empty());
}

fn repr_cases() {
    let (messages, _guard) = support::capture();
    let cases = [
        (r#"{"service_tier": "it's"}"#, r#""it's""#),
        (r#"{"service_tier": "a'b\"c"}"#, r#"'a\'b"c'"#),
        (r#"{"service_tier": "x\\y\n\t\r"}"#, r#"'x\\y\n\t\r'"#),
    ];
    for (source, shown) in cases {
        assert_eq!(drop_tier(&support::object(source)), support::object("{}"));
        let warned = messages.at(Level::WARN).pop().expect("a warning");
        assert_eq!(
            warned,
            format!(
                "Bedrock Mantle Responses API: dropping unsupported service_tier {shown} (supported: ['auto', 'default'])."
            )
        );
    }
}

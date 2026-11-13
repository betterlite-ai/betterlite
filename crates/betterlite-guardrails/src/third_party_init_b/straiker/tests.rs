#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tracing::{Event, Subscriber, subscriber::with_default};
use tracing_subscriber::{
    Layer,
    layer::{Context, SubscriberExt as _},
};

use super::initialize_straiker;
use crate::third_party_init_b::error::ThirdPartyInitBError;

fn params(extras: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "straiker", "mode": "pre_call"{extras}}}"#);
    decode_litellm_params(body.as_bytes()).unwrap()
}

struct Warnings(Arc<AtomicUsize>);

impl<Inner> Layer<Inner> for Warnings
where
    Inner: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() == tracing::Level::WARN
            && event.metadata().target().contains("third_party_init_b")
        {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[test]
fn straiker_refuses_a_missing_or_empty_api_key() {
    let missing = initialize_straiker(&params(""), None).unwrap_err();
    assert!(matches!(
        missing,
        ThirdPartyInitBError::MissingStraikerApiKey
    ));
    assert_eq!(missing.to_string(), "api_key is required for straiker");
    let empty = initialize_straiker(&params(r#", "api_key": """#), None).unwrap_err();
    assert!(matches!(empty, ThirdPartyInitBError::MissingStraikerApiKey));
}

#[test]
fn straiker_copies_the_config_values() {
    let built = initialize_straiker(
        &params(
            r#",
            "api_key": "sk_agent_key",
            "api_base": "https://straiker.test",
            "source": "Configured Source",
            "timeout": 3,
            "max_retries": 5,
            "initial_backoff": 0.2,
            "max_backoff": 4.0,
            "unreachable_fallback": "fail_open",
            "fail_on_error": false,
            "max_payload_bytes": 1024,
            "custom_headers": {"x-header": "value"},
            "metadata": {"team": "safety"},
            "verbose": true,
            "api_version": "v3",
            "agent_ref": "agent-1",
            "client": "cli-1",
            "format_hint": "openai.chat",
            "default_on": true
            "#,
        ),
        Some("straiker-check"),
    )
    .unwrap();
    assert_eq!(built.name, "straiker-check");
    assert_eq!(built.api_key.expose_secret(), "sk_agent_key");
    assert_eq!(built.api_base, "https://straiker.test");
    assert_eq!(built.source, "Configured Source");
    assert_eq!(built.timeout, Some(3.0_f64));
    assert_eq!(built.max_retries, Some(5));
    assert_eq!(built.initial_backoff, Some(0.2_f64));
    assert_eq!(built.max_backoff, Some(4.0_f64));
    assert_eq!(built.unreachable_fallback, "fail_open");
    assert!(!built.fail_on_error);
    assert_eq!(built.max_payload_bytes, Some(1024));
    assert_eq!(
        built.custom_headers,
        Some(BTreeMap::from([(
            "x-header".to_owned(),
            "value".to_owned()
        )]))
    );
    assert_eq!(
        built.metadata,
        Some(BTreeMap::from([("team".to_owned(), "safety".to_owned())]))
    );
    assert_eq!(built.verbose, Some(true));
    assert_eq!(built.api_version.as_deref(), Some("v3"));
    assert_eq!(built.agent_ref.as_deref(), Some("agent-1"));
    assert_eq!(built.client.as_deref(), Some("cli-1"));
    assert_eq!(built.format_hint.as_deref(), Some("openai.chat"));
    assert!(built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn straiker_applies_the_defaults() {
    let built = initialize_straiker(&params(r#", "api_key": "sk_agent_key""#), None).unwrap();
    assert_eq!(built.name, "straiker");
    assert_eq!(built.api_base, "https://api.prod.straiker.ai");
    assert_eq!(built.source, "LiteLLM Gateway");
    assert_eq!(built.timeout, None);
    assert_eq!(built.max_retries, None);
    assert_eq!(built.initial_backoff, None);
    assert_eq!(built.max_backoff, None);
    assert_eq!(built.unreachable_fallback, "fail_closed");
    assert!(built.fail_on_error);
    assert_eq!(built.max_payload_bytes, None);
    assert_eq!(built.custom_headers, None);
    assert_eq!(built.metadata, None);
    assert_eq!(built.verbose, None);
    assert_eq!(built.api_version, None);
    assert_eq!(built.agent_ref, None);
    assert_eq!(built.client, None);
    assert_eq!(built.format_hint, None);
    assert!(!built.default_on);
}

#[rstest]
#[case::default_app_wins(r#", "default_app": "Default App", "source": "Source""#, "Default App")]
#[case::empty_default_app_falls_through(r#", "default_app": "", "source": "Source""#, "Source")]
#[case::no_source_uses_the_default(r#""#, "LiteLLM Gateway")]
#[case::non_string_default_app_falls_through(r#", "default_app": 5, "source": "Source""#, "Source")]
fn straiker_source_reads_default_app_then_source(#[case] extras: &str, #[case] expected: &str) {
    let key = r#", "api_key": "sk_agent_key""#;
    let built = initialize_straiker(&params(&format!("{key}{extras}")), None).unwrap();
    assert_eq!(built.source, expected);
}

#[test]
fn straiker_optional_params_win_and_drop_nulls() {
    let built = initialize_straiker(
        &params(
            r#",
            "api_key": "sk_agent_key",
            "timeout": 9,
            "max_retries": 1,
            "fail_on_error": false,
            "unreachable_fallback": "fail_open",
            "optional_params": {
                "timeout": 2.5,
                "max_retries": null,
                "fail_on_error": true,
                "unreachable_fallback": null
            }
            "#,
        ),
        None,
    )
    .unwrap();
    assert_eq!(built.timeout, Some(2.5_f64));
    assert_eq!(built.max_retries, Some(1));
    assert!(built.fail_on_error);
    assert_eq!(built.unreachable_fallback, "fail_open");
}

#[rstest]
#[case::v1("v1", 0, Some("v1"))]
#[case::v3("v3", 0, Some("v3"))]
#[case::unknown("banana", 1, None)]
#[case::empty("", 1, None)]
fn straiker_checks_the_api_version(
    #[case] version: &str,
    #[case] warnings: usize,
    #[case] expected: Option<&str>,
) {
    let counter = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(Warnings(Arc::clone(&counter)));
    let extras = format!(r#", "api_key": "sk_agent_key", "api_version": "{version}""#);
    let built = with_default(subscriber, || {
        initialize_straiker(&params(&extras), None).unwrap()
    });
    assert_eq!(counter.load(Ordering::Relaxed), warnings);
    assert_eq!(built.api_version.as_deref(), expected);
}

#[rstest]
#[case::anthropic_messages("anthropic.messages")]
#[case::openai_chat("openai.chat")]
fn straiker_keeps_a_known_format_hint(#[case] hint: &str) {
    let extras = format!(r#", "api_key": "sk_agent_key", "format_hint": "{hint}""#);
    let built = initialize_straiker(&params(&extras), None).unwrap();
    assert_eq!(built.format_hint.as_deref(), Some(hint));
}

#[test]
fn straiker_refuses_an_unknown_format_hint() {
    let error = initialize_straiker(
        &params(r#", "api_key": "sk_agent_key", "format_hint": "text.completion""#),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ThirdPartyInitBError::InvalidFormatHint { .. }
    ));
    assert!(error.to_string().contains("text.completion"));
}

#[test]
fn straiker_api_base_falls_back_when_empty() {
    let built = initialize_straiker(
        &params(r#", "api_key": "sk_agent_key", "api_base": """#),
        None,
    )
    .unwrap();
    assert_eq!(built.api_base, "https://api.prod.straiker.ai");
}

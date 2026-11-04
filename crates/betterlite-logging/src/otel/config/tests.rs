#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use super::{
    CaptureMessageContent, OtelDestination, OtlpHttpTls, Settings, SpanScope, resolve_otlp_http_tls,
};
use betterlite_config::loader::ConfigLoader;

use rstest::rstest;

#[test]
fn destination_headers_are_percent_encoded_and_cache_key_ignores_scope() {
    let headers = BTreeMap::from([
        ("authorization".to_owned(), "Basic a,b==".to_owned()),
        ("x".to_owned(), "a=b".to_owned()),
    ]);
    let first = OtelDestination::new(
        "https://collector/v1".to_owned(),
        headers.clone(),
        BTreeMap::from([("service.name".to_owned(), "proxy".to_owned())]),
        Some("langfuse_otel".to_owned()),
        Some("grpc".to_owned()),
        SpanScope::Full,
    );
    let second = OtelDestination::new(
        "https://collector/v1".to_owned(),
        headers,
        BTreeMap::from([("service.name".to_owned(), "proxy".to_owned())]),
        Some("langfuse_otel".to_owned()),
        Some("grpc".to_owned()),
        SpanScope::LlmOnly,
    );
    assert_eq!(
        first.header_string(),
        "authorization=Basic%20a%2Cb%3D%3D,x=a%3Db"
    );
    assert_eq!(first.cache_key(), second.cache_key());
}

#[rstest]
#[case::no_content("no_content", false)]
#[case::span_only("span_only", true)]
#[case::event_only("event_only", false)]
#[case::span_and_event("span_and_event", true)]
fn capture_content_is_enabled_only_for_span_modes(#[case] mode: &str, #[case] expected: bool) {
    let yaml = format!("callback_settings:\n  otel:\n    capture_message_content: {mode}\n");
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&yaml)
    .expect("valid config");
    let settings = Settings::from_config(&config).expect("valid OTel settings");
    assert_eq!(settings.capture_span_content(), expected);
}

#[rstest]
#[case::verify_true(&[], OtlpHttpTls::DefaultVerified)]
#[case::verify_false(&[("SSL_VERIFY", "false")], OtlpHttpTls::NoVerify)]
#[case::certificate_path(&[("SSL_VERIFY", "/etc/ssl/custom.pem")], OtlpHttpTls::CertificateFile("/etc/ssl/custom.pem".to_owned()))]
fn resolves_tls_policy(#[case] values: &[(&str, &str)], #[case] expected: OtlpHttpTls) {
    let environment: BTreeMap<String, String> = values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .expect("valid config");
    assert_eq!(
        resolve_otlp_http_tls(&config, &environment).expect("valid TLS"),
        expected
    );
}

#[test]
fn capture_mode_conversion_is_typed() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("callback_settings:\n  otel:\n    capture_message_content: EVENT_ONLY\n")
    .expect("valid config");
    assert_eq!(
        Settings::from_config(&config)
            .expect("valid OTel settings")
            .capture_message_content,
        CaptureMessageContent::EventOnly
    );
}

#[test]
fn normalizes_supported_service_exclusion_spellings_and_drops_unknown_names() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("callback_settings:\n  otel:\n    excluded_services: [redis, postgres, postgresql, batch_write_to_db, redis_spend, typo]\n")
    .expect("valid config");
    let settings = Settings::from_config(&config).expect("valid OTel settings");
    assert_eq!(
        settings.excluded_services,
        BTreeSet::from(["postgresql".to_owned(), "redis".to_owned()])
    );
}

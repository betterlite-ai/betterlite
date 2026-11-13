#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use super::destination::{DestinationFields, OtelDestination, SpanScope};
use super::{
    CaptureMessageContent, ExporterOwner, OtlpHttpTls, Settings, Signal, resolve_otlp_http_tls,
};
use betterlite_config::loader::ConfigLoader;

use rstest::rstest;

#[test]
fn destination_headers_are_percent_encoded_and_cache_key_ignores_scope() {
    let headers = BTreeMap::from([
        ("authorization".to_owned(), "Basic a,b==".to_owned()),
        ("x".to_owned(), "a=b-_.~/\u{e9}".to_owned()),
    ]);
    let first = OtelDestination::from(DestinationFields {
        endpoint: "https://collector/v1".to_owned(),
        headers: headers.clone(),
        resource_attributes: BTreeMap::from([("service.name".to_owned(), "proxy".to_owned())]),
        callback_name: Some("langfuse_otel".to_owned()),
        protocol: Some("grpc".to_owned()),
        span_scope: SpanScope::Full,
    });
    let second = OtelDestination::from(DestinationFields {
        endpoint: "https://collector/v1".to_owned(),
        headers,
        resource_attributes: BTreeMap::from([("service.name".to_owned(), "proxy".to_owned())]),
        callback_name: Some("langfuse_otel".to_owned()),
        protocol: Some("grpc".to_owned()),
        span_scope: SpanScope::LlmOnly,
    });
    assert_eq!(
        first.header_string(),
        "authorization=Basic%20a%2Cb%3D%3D,x=a%3Db-_.~%2F%C3%A9"
    );
    assert_eq!(first.cache_key(), second.cache_key());
    assert_eq!(
        first.cache_key(),
        (
            "https://collector/v1".to_owned(),
            vec![
                ("authorization".to_owned(), "Basic a,b==".to_owned()),
                ("x".to_owned(), "a=b-_.~/\u{e9}".to_owned())
            ],
            vec![("service.name".to_owned(), "proxy".to_owned())],
            Some("grpc".to_owned()),
        )
    );
    assert_eq!(second.as_ref().span_scope, SpanScope::LlmOnly);
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
        resolve_otlp_http_tls(&config, &environment, Signal::Traces).expect("valid TLS"),
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
fn unrecognized_capture_mode_falls_back_to_no_content_instead_of_erroring() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("callback_settings:\n  otel:\n    capture_message_content: banana\n")
    .expect("valid config");
    let settings = Settings::from_config(&config).expect("capture_message_content is a plain str in LiteLLM, so an unrecognized value loads rather than failing config");
    assert_eq!(
        settings.capture_message_content,
        CaptureMessageContent::NoContent
    );
    assert!(!settings.capture_span_content());
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

#[rstest]
#[case::full("full", SpanScope::Full)]
#[case::llm_only("llm_only", SpanScope::LlmOnly)]
fn maps_span_scope(#[case] value: &str, #[case] expected: SpanScope) {
    let yaml = format!("callback_settings:\n  otel:\n    langfuse_span_scope: {value}\n");
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&yaml)
    .expect("valid config");
    assert_eq!(
        Settings::from_config(&config)
            .expect("valid OTel settings")
            .langfuse_span_scope,
        expected
    );
}

#[rstest]
#[case::arize("arize", ExporterOwner::ArizeAx)]
#[case::arize_phoenix("arize_phoenix", ExporterOwner::ArizePhoenix)]
#[case::langfuse("langfuse_otel", ExporterOwner::LangfuseOtel)]
#[case::weave("weave_otel", ExporterOwner::WeaveOtel)]
#[case::levo("levo", ExporterOwner::Levo)]
#[case::agentops("agentops", ExporterOwner::Agentops)]
#[case::newrelic("newrelic", ExporterOwner::Newrelic)]
#[case::signoz("signoz", ExporterOwner::Signoz)]
fn maps_exporter_owner_names(#[case] name: &str, #[case] expected: ExporterOwner) {
    let yaml =
        format!("callback_settings:\n  otel:\n    exporters: [{{kind: custom, owner: {name}}}]\n");
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&yaml)
    .expect("valid config");
    let settings = Settings::from_config(&config).expect("valid OTel settings");
    assert_eq!(settings.exporters[0].owner, Some(expected));
}

#[rstest]
#[case::global(
    "OTEL_EXPORTER_OTLP_CERTIFICATE",
    "/ca.pem",
    OtlpHttpTls::DefaultVerified
)]
#[case::traces(
    "OTEL_EXPORTER_OTLP_TRACES_CERTIFICATE",
    "/ca.pem",
    OtlpHttpTls::DefaultVerified
)]
#[case::empty("OTEL_EXPORTER_OTLP_CERTIFICATE", "", OtlpHttpTls::NoVerify)]
fn exporter_certificate_precedence(
    #[case] name: &str,
    #[case] value: &str,
    #[case] expected: OtlpHttpTls,
) {
    let environment = BTreeMap::from([
        ("SSL_VERIFY".to_owned(), "false".to_owned()),
        (name.to_owned(), value.to_owned()),
    ]);
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .expect("config");
    assert_eq!(
        resolve_otlp_http_tls(&config, &environment, Signal::Traces).expect("TLS"),
        expected
    );
}

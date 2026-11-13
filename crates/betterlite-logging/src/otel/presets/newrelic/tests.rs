#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as _;

use betterlite_env::EnvError;
use rstest::rstest;

use super::{
    NewrelicPreset, NewrelicPresetError, newrelic_dynamic_endpoint, newrelic_dynamic_headers,
    newrelic_endpoint_for_region,
};
use crate::otel::config::destination::SpanScope;
use crate::otel::config::{CaptureMessageContent, ExporterOwner, ExporterSpec, Settings};
use crate::otel::presets::base::{Preset as _, PresetError};

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    environment(variables)
}

fn base_settings(capture: CaptureMessageContent) -> Settings {
    Settings {
        exporters: vec![ExporterSpec {
            kind: "console".to_owned(),
            endpoint: None,
            traces_endpoint: None,
            headers: None,
            owner: None,
            options: None,
            use_simple_processor: None,
            requires_headers: false,
        }],
        service_name: "litellm".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: capture,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: vec!["genai".to_owned(), "openinference".to_owned()],
        resource_attributes: BTreeMap::from([("service.name".to_owned(), "proxy".to_owned())]),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

fn build(
    variables: &[(&str, &str)],
    capture: CaptureMessageContent,
) -> Result<Settings, PresetError> {
    NewrelicPreset.build(&environment(variables), base_settings(capture), false)
}

#[rstest]
#[case::unset(&[], "https://otlp.nr-data.net")]
#[case::us(&[("NEW_RELIC_REGION", "us")], "https://otlp.nr-data.net")]
#[case::eu(&[("NEW_RELIC_REGION", "eu")], "https://otlp.eu01.nr-data.net")]
#[case::upper_case_eu(&[("NEW_RELIC_REGION", "EU")], "https://otlp.eu01.nr-data.net")]
#[case::unknown(&[("NEW_RELIC_REGION", "ap-southeast-2")], "https://otlp.nr-data.net")]
#[case::empty(&[("NEW_RELIC_REGION", "")], "https://otlp.nr-data.net")]
fn preset_picks_the_region_endpoint(#[case] variables: &[(&str, &str)], #[case] endpoint: &str) {
    let settings = build(variables, CaptureMessageContent::NoContent).expect("the preset builds");
    assert_eq!(settings.exporters.len(), 2);
    assert_eq!(settings.exporters[1].kind, "otlp_http");
    assert_eq!(settings.exporters[1].endpoint.as_deref(), Some(endpoint));
}

#[test]
fn preset_sets_the_key_header_and_always_requires_headers() {
    let settings = build(
        &[("NEW_RELIC_LICENSE_KEY", "license-1")],
        CaptureMessageContent::NoContent,
    )
    .expect("the preset builds");
    assert_eq!(
        settings.exporters[1],
        ExporterSpec {
            kind: "otlp_http".to_owned(),
            endpoint: Some("https://otlp.nr-data.net".to_owned()),
            traces_endpoint: None,
            headers: Some("api-key=license-1".to_owned()),
            owner: Some(ExporterOwner::Newrelic),
            options: None,
            use_simple_processor: None,
            requires_headers: true,
        }
    );
}

#[rstest]
#[case::missing(&[])]
#[case::empty(&[("NEW_RELIC_LICENSE_KEY", "")])]
fn preset_omits_the_header_but_still_requires_it(#[case] variables: &[(&str, &str)]) {
    let settings = build(variables, CaptureMessageContent::NoContent).expect("the preset builds");
    assert_eq!(settings.exporters[1].headers, None);
    assert!(settings.exporters[1].requires_headers);
}

#[test]
fn preset_appends_the_genai_mapper_once() {
    let settings = build(&[], CaptureMessageContent::NoContent).expect("the preset builds");
    assert_eq!(settings.mapper_names, ["genai", "openinference"]);
    let mut base = base_settings(CaptureMessageContent::NoContent);
    base.mapper_names = vec!["openinference".to_owned()];
    let built = NewrelicPreset
        .build(&environment(&[]), base, false)
        .expect("the preset builds");
    assert_eq!(built.mapper_names, ["openinference", "genai"]);
}

#[rstest]
#[case::one("1", true)]
#[case::true_word("true", true)]
#[case::t("t", true)]
#[case::upper_t("T", true)]
#[case::yes("yes", true)]
#[case::y("y", true)]
#[case::on("on", true)]
#[case::zero("0", false)]
#[case::false_word("false", false)]
#[case::f("f", false)]
#[case::upper_f("F", false)]
#[case::no("no", false)]
#[case::n("n", false)]
#[case::off("off", false)]
fn preset_sets_the_capture_mode_from_the_flag(#[case] value: &str, #[case] span_only: bool) {
    let settings = build(
        &[("NEW_RELIC_AI_MONITORING_RECORD_CONTENT_ENABLED", value)],
        CaptureMessageContent::SpanAndEvent,
    )
    .expect("the preset builds");
    let expected = if span_only {
        CaptureMessageContent::SpanOnly
    } else {
        CaptureMessageContent::NoContent
    };
    assert_eq!(settings.capture_message_content, expected);
}

#[test]
fn preset_keeps_the_base_capture_mode_when_the_flag_is_unset() {
    let settings = build(&[], CaptureMessageContent::EventOnly).expect("the preset builds");
    assert_eq!(
        settings.capture_message_content,
        CaptureMessageContent::EventOnly
    );
}

#[derive(Clone, Copy)]
enum Rejection {
    Value,
    Parser,
}

#[rstest]
#[case::empty("", Rejection::Value)]
#[case::padded(" true ", Rejection::Value)]
#[case::word("sometimes", Rejection::Parser)]
#[case::number("2", Rejection::Parser)]
fn preset_rejects_an_unparsable_record_content_flag(
    #[case] value: &str,
    #[case] rejection: Rejection,
) {
    let error = build(
        &[("NEW_RELIC_AI_MONITORING_RECORD_CONTENT_ENABLED", value)],
        CaptureMessageContent::NoContent,
    )
    .expect_err("an unparsable flag fails the preset");
    let matched = match rejection {
        Rejection::Value => matches!(
            &error,
            PresetError::Newrelic(NewrelicPresetError::RecordContent { value: rejected })
                if rejected.as_str() == value
        ),
        Rejection::Parser => matches!(
            &error,
            PresetError::Newrelic(NewrelicPresetError::RecordContentEnvironment(_))
        ),
    };
    assert!(matched, "unexpected error {error:?}");
}

#[test]
fn record_content_error_keeps_the_parser_cause() {
    let error = build(
        &[(
            "NEW_RELIC_AI_MONITORING_RECORD_CONTENT_ENABLED",
            "sometimes",
        )],
        CaptureMessageContent::NoContent,
    )
    .expect_err("an unparsable flag fails the preset");
    let source = error
        .source()
        .expect("the parse error is kept as the source");
    assert!(matches!(
        source.downcast_ref::<EnvError>(),
        Some(EnvError::Bool { name }) if name == "NEW_RELIC_AI_MONITORING_RECORD_CONTENT_ENABLED"
    ));
}

#[rstest]
#[case::us("us", Some("https://otlp.nr-data.net"))]
#[case::us_upper("US", Some("https://otlp.nr-data.net"))]
#[case::eu("eu", Some("https://otlp.eu01.nr-data.net"))]
#[case::mixed_case("Eu", Some("https://otlp.eu01.nr-data.net"))]
#[case::unknown("asia", None)]
#[case::empty("", None)]
fn region_table_lookup(#[case] region: &str, #[case] expected: Option<&str>) {
    assert_eq!(newrelic_endpoint_for_region(region), expected);
}

#[rstest]
#[case::present(&[("newrelic_api_key", "key-1")], Some("key-1"))]
#[case::empty(&[("newrelic_api_key", "")], None)]
#[case::missing(&[], None)]
#[case::other_parameter(&[("newrelic_region", "eu")], None)]
fn dynamic_headers_carry_only_a_non_empty_key(
    #[case] variables: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let headers = newrelic_dynamic_headers(&params(variables));
    assert_eq!(headers.get("api-key").map(String::as_str), expected);
    assert_eq!(headers.len(), usize::from(expected.is_some()));
}

#[rstest]
#[case::unset(&[], "https://otlp.nr-data.net")]
#[case::empty(&[("newrelic_region", "")], "https://otlp.nr-data.net")]
#[case::eu(&[("newrelic_region", "eu")], "https://otlp.eu01.nr-data.net")]
#[case::upper_eu(&[("newrelic_region", "EU")], "https://otlp.eu01.nr-data.net")]
#[case::unknown(&[("newrelic_region", "mars")], "https://otlp.nr-data.net")]
fn dynamic_endpoint_uses_the_team_region_or_us(
    #[case] variables: &[(&str, &str)],
    #[case] expected: &str,
) {
    assert_eq!(newrelic_dynamic_endpoint(&params(variables)), expected);
}

#[test]
fn dynamic_endpoint_ignores_other_parameters() {
    let given = params(&[("newrelic_api_key", "key-1")]);
    assert_eq!(
        newrelic_dynamic_endpoint(&given),
        "https://otlp.nr-data.net"
    );
}

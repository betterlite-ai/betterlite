#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::{
    dynamic_otlp_endpoint, dynamic_otlp_headers, dynamic_params_map, preset_for,
    project_routing_headers,
};
use crate::otel::config::destination::SpanScope;
use crate::otel::config::{CaptureMessageContent, ExporterOwner, Settings};

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn allowlist(entries: &[&str]) -> Vec<AllowedDestination> {
    entries
        .iter()
        .filter_map(|entry| AllowedDestination::parse(entry))
        .collect()
}

fn base_settings() -> Settings {
    Settings {
        exporters: Vec::new(),
        service_name: "litellm".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::NoContent,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: Vec::new(),
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

fn params(entries: &[(&str, Json)]) -> Json {
    Json::Object(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<IndexMap<String, Json>>(),
    )
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[rstest]
#[case::agentops("agentops", &[], ExporterOwner::Agentops)]
#[case::arize("arize", &[], ExporterOwner::ArizeAx)]
#[case::phoenix("arize_phoenix", &[], ExporterOwner::ArizePhoenix)]
#[case::langfuse(
    "langfuse_otel",
    &[("LANGFUSE_PUBLIC_KEY", "pk"), ("LANGFUSE_SECRET_KEY", "sk")],
    ExporterOwner::LangfuseOtel
)]
#[case::levo(
    "levo",
    &[
        ("LEVOAI_API_KEY", "key"),
        ("LEVOAI_ORG_ID", "org"),
        ("LEVOAI_WORKSPACE_ID", "workspace"),
        ("LEVOAI_COLLECTOR_URL", "https://levo.example"),
    ],
    ExporterOwner::Levo
)]
#[case::newrelic("newrelic", &[], ExporterOwner::Newrelic)]
#[case::signoz("signoz", &[], ExporterOwner::Signoz)]
#[case::weave(
    "weave_otel",
    &[("WANDB_API_KEY", "key"), ("WANDB_PROJECT_ID", "entity/project")],
    ExporterOwner::WeaveOtel
)]
fn preset_for_returns_the_named_preset(
    #[case] callback_name: &str,
    #[case] variables: &[(&str, &str)],
    #[case] owner: ExporterOwner,
) {
    let preset = preset_for(callback_name).expect("the callback has a preset");
    let settings = preset
        .build(&environment(variables), base_settings(), false)
        .expect("the preset builds");
    assert_eq!(
        settings.exporters.last().and_then(|spec| spec.owner),
        Some(owner)
    );
}

#[test]
fn preset_for_maps_langtrace_to_its_mapper_only_preset() {
    let preset = preset_for("langtrace").expect("langtrace has a preset");
    let settings = preset
        .build(&environment(&[]), base_settings(), false)
        .expect("the preset builds");
    assert!(settings.exporters.is_empty());
    assert_eq!(settings.mapper_names, ["langtrace"]);
}

#[rstest]
#[case::unknown("datadog")]
#[case::empty("")]
#[case::wrong_case("AgentOps")]
#[case::trailing_space("agentops ")]
fn preset_for_rejects_other_names(#[case] callback_name: &str) {
    assert!(preset_for(callback_name).is_none());
}

#[rstest]
#[case::arize(
    "arize",
    &[("arize_space_id", text("space-1")), ("arize_api_key", text("key-1"))],
    &[]
)]
#[case::langfuse(
    "langfuse_otel",
    &[("langfuse_public_key", text("pk")), ("langfuse_secret_key", text("sk"))],
    &[]
)]
#[case::newrelic("newrelic", &[("newrelic_api_key", text("key-1"))], &[])]
#[case::signoz(
    "signoz",
    &[
        ("signoz_ingestion_endpoint", text("https://collector.example/v1/traces")),
        ("signoz_ingestion_key", text("key-1")),
    ],
    &["collector.example"]
)]
#[case::weave(
    "weave_otel",
    &[
        ("wandb_api_key", text("key-1")),
        ("weave_project_id", text("entity/project")),
    ],
    &[]
)]
fn dynamic_headers_come_from_the_callbacks_builder(
    #[case] callback_name: &str,
    #[case] entries: &[(&str, Json)],
    #[case] allowed: &[&str],
) {
    let headers = dynamic_otlp_headers(Some(callback_name), &params(entries), &allowlist(allowed))
        .expect("the callback has a header builder");
    assert!(!headers.is_empty());
}

#[rstest]
#[case::agentops("agentops")]
#[case::phoenix("arize_phoenix")]
#[case::langtrace("langtrace")]
#[case::levo("levo")]
#[case::unknown("datadog")]
fn dynamic_headers_are_none_without_a_builder(#[case] callback_name: &str) {
    let given = params(&[("arize_api_key", text("key-1"))]);
    assert_eq!(
        dynamic_otlp_headers(Some(callback_name), &given, &allowlist(&[])),
        None
    );
}

#[test]
fn dynamic_headers_are_none_without_a_callback_name() {
    let given = params(&[("arize_api_key", text("key-1"))]);
    assert_eq!(dynamic_otlp_headers(None, &given, &allowlist(&[])), None);
}

#[rstest]
#[case::null(Json::Null)]
#[case::empty_object(Json::Object(IndexMap::new()))]
#[case::only_non_string_values(params(&[("arize_api_key", Json::Bool(true))]))]
#[case::list(Json::List(vec![text("arize_api_key")]))]
#[case::text(Json::Text("arize_api_key".to_owned()))]
fn dynamic_headers_are_none_without_string_parameters(#[case] given: Json) {
    assert_eq!(
        dynamic_otlp_headers(Some("arize"), &given, &allowlist(&[])),
        None
    );
}

#[test]
fn dynamic_headers_drop_values_that_are_not_strings() {
    let given = params(&[
        ("arize_api_key", text("key-1")),
        (
            "arize_space_id",
            Json::Number(serde_json::Number::from(7_u64)),
        ),
    ]);
    let headers = dynamic_otlp_headers(Some("arize"), &given, &allowlist(&[]))
        .expect("the api key is a string");
    assert_eq!(
        headers,
        BTreeMap::from([("api_key".to_owned(), "key-1".to_owned())])
    );
}

#[test]
fn dynamic_headers_are_none_when_the_builder_returns_nothing() {
    let given = params(&[("langfuse_public_key", text("pk"))]);
    assert_eq!(
        dynamic_otlp_headers(Some("langfuse_otel"), &given, &allowlist(&[])),
        None
    );
}

#[test]
fn signoz_dynamic_headers_see_the_operator_allowlist() {
    let given = params(&[
        (
            "signoz_ingestion_endpoint",
            text("https://collector.example/v1/traces"),
        ),
        ("signoz_ingestion_key", text("key-1")),
    ]);
    assert_eq!(
        dynamic_otlp_headers(Some("signoz"), &given, &allowlist(&["other.example"])),
        None
    );
    let headers = dynamic_otlp_headers(Some("signoz"), &given, &allowlist(&["collector.example"]))
        .expect("the endpoint is allowlisted");
    assert_eq!(
        headers,
        BTreeMap::from([("signoz-ingestion-key".to_owned(), "key-1".to_owned())])
    );
}

#[rstest]
#[case::newrelic("newrelic", "https://otlp.nr-data.net")]
#[case::newrelic_eu("newrelic", "https://otlp.eu01.nr-data.net")]
#[case::signoz("signoz", "https://collector.example/v1/traces")]
fn dynamic_endpoint_answers_the_team_region(#[case] callback_name: &str, #[case] expected: &str) {
    let entries: &[(&str, Json)] = if callback_name == "newrelic" {
        &[(
            "newrelic_region",
            text(if expected.contains("eu01") {
                "eu"
            } else {
                "us"
            }),
        )]
    } else {
        &[
            (
                "signoz_ingestion_endpoint",
                text("https://collector.example/v1/traces"),
            ),
            ("signoz_ingestion_key", text("key-1")),
        ]
    };
    let hosts = allowlist(&["collector.example"]);
    assert_eq!(
        dynamic_otlp_endpoint(Some(callback_name), &params(entries), &hosts).as_deref(),
        Some(expected)
    );
}

#[rstest]
#[case::arize("arize")]
#[case::langfuse("langfuse_otel")]
#[case::weave("weave_otel")]
#[case::agentops("agentops")]
#[case::unknown("datadog")]
fn dynamic_endpoint_is_none_without_a_resolver(#[case] callback_name: &str) {
    let given = params(&[("newrelic_region", text("eu"))]);
    assert_eq!(
        dynamic_otlp_endpoint(Some(callback_name), &given, &allowlist(&[])),
        None
    );
    assert_eq!(dynamic_otlp_endpoint(None, &given, &allowlist(&[])), None);
}

#[rstest]
#[case::null(Json::Null)]
#[case::empty_object(Json::Object(IndexMap::new()))]
fn dynamic_endpoint_is_none_without_parameters(#[case] given: Json) {
    assert_eq!(
        dynamic_otlp_endpoint(Some("newrelic"), &given, &allowlist(&[])),
        None
    );
}

#[rstest]
#[case::project("phoenix_project_name", "project-1")]
#[case::override_key("phoenix_project_name_override", "project-2")]
fn project_routing_headers_answer_for_phoenix(#[case] key: &str, #[case] project: &str) {
    let metadata = BTreeMap::from([(key.to_owned(), format!(" {project} "))]);
    let headers = project_routing_headers(Some("arize_phoenix"), Some(&metadata));
    assert_eq!(
        headers,
        BTreeMap::from([("x-project-name".to_owned(), project.to_owned())])
    );
}

#[rstest]
#[case::other_callback("arize")]
#[case::unknown("datadog")]
fn project_routing_headers_ignore_other_callbacks(#[case] callback_name: &str) {
    let metadata = BTreeMap::from([("phoenix_project_name".to_owned(), "project".to_owned())]);
    assert!(project_routing_headers(Some(callback_name), Some(&metadata)).is_empty());
    assert!(project_routing_headers(None, Some(&metadata)).is_empty());
}

#[test]
fn project_routing_headers_are_empty_without_metadata() {
    assert!(project_routing_headers(Some("arize_phoenix"), None).is_empty());
}

#[test]
fn dynamic_params_map_keeps_only_string_values() {
    let given = params(&[
        ("text", text("value")),
        ("flag", Json::Bool(true)),
        ("number", Json::Number(serde_json::Number::from(1_u64))),
        ("null", Json::Null),
        ("list", Json::List(Vec::new())),
        ("object", Json::Object(IndexMap::new())),
    ]);
    assert_eq!(
        dynamic_params_map(&given),
        BTreeMap::from([("text".to_owned(), "value".to_owned())])
    );
}

#[test]
fn dynamic_params_map_is_empty_for_a_non_object() {
    assert!(dynamic_params_map(&Json::List(Vec::new())).is_empty());
    assert!(dynamic_params_map(&Json::Null).is_empty());
}

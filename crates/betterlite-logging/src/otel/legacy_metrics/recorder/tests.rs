#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::FixedOffset;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, MetricData, ResourceMetrics, ScopeMetrics,
};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use rstest::rstest;
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;

use super::{LegacyMetricRecorder, MetricCall};
use crate::otel::legacy_metrics::filter::CallbackSettingsView;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::vocabulary::semconv::OTELSemconvCategory;
use crate::otel_provider_assembly::metric_instruments::create_instruments;

const DURATION: &str = "gen_ai.client.operation.duration";
const TOKENS: &str = "gen_ai.client.token.usage";
const COST: &str = "gen_ai.usage.cost";
const TTFT: &str = "gen_ai.server.time_to_first_token";
const TPOT: &str = "gen_ai.server.time_per_output_token";
const RESPONSE: &str = "gen_ai.client.response.duration";
const BASE_KWARGS: &str = r#"{
    "model": "gpt-4o",
    "call_type": "acompletion",
    "litellm_params": {"custom_llm_provider": "openai", "metadata": {}},
    "standard_logging_object": {
        "metadata": {"user_api_key_alias": "key-a"},
        "hidden_params": {"api_base": "https://x"}
    }
}"#;
const USAGE_2_4: &str = r#"{"usage": {"prompt_tokens": 2, "completion_tokens": 4}}"#;
const USAGE_1_4: &str = r#"{"usage": {"prompt_tokens": 1, "completion_tokens": 4}}"#;

#[derive(Debug)]
struct Recorded {
    name: String,
    attributes: BTreeMap<String, String>,
    count: u64,
    sum: String,
}

struct NoSettings;

impl CallbackSettingsView for NoSettings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        None
    }
}

fn typed<T>(text: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(text).expect("typed value")
}

fn base_config() -> OpenTelemetryConfig {
    OpenTelemetryConfig::from_env(&BTreeMap::new())
}

fn kwargs(extra: &str) -> MetadataMap {
    let mut base: MetadataMap = typed(BASE_KWARGS);
    base.extend(typed::<MetadataMap>(extra));
    base
}

fn build_recorder(
    config: &OpenTelemetryConfig,
    callback_name: Option<&str>,
    exporter: &InMemoryMetricExporter,
) -> (LegacyMetricRecorder, SdkMeterProvider) {
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    let recorder = LegacyMetricRecorder::new(
        create_instruments(&provider.meter("legacy-metrics-test")),
        config,
        callback_name.map(str::to_owned),
        Arc::new(NoSettings),
        FixedOffset::east_opt(0).expect("utc"),
    )
    .expect("recorder");
    (recorder, provider)
}

fn call_of<'call>(
    kwargs: &'call MetadataMap,
    response: Option<&'call OtelModelValue>,
) -> MetricCall<'call> {
    MetricCall {
        kwargs,
        response,
        start_time: Decimal::from(10_i64),
        end_time: Decimal::from(13_i64),
    }
}

fn record_with(
    config: &OpenTelemetryConfig,
    callback_name: Option<&str>,
    extra: &str,
    response: Option<&str>,
) -> Vec<Recorded> {
    let exporter = InMemoryMetricExporter::default();
    let (recorder, provider) = build_recorder(config, callback_name, &exporter);
    let call_kwargs = kwargs(extra);
    let typed_response: Option<OtelModelValue> = response.map(typed);
    recorder
        .record_metrics(&call_of(&call_kwargs, typed_response.as_ref()))
        .expect("record");
    provider.force_flush().expect("flush");
    let finished = exporter.get_finished_metrics().expect("finished metrics");
    let mut recorded = Vec::new();
    for metric in finished
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .flat_map(ScopeMetrics::metrics)
    {
        let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = metric.data() else {
            continue;
        };
        for point in histogram.data_points() {
            recorded.push(Recorded {
                name: metric.name().to_owned(),
                attributes: point
                    .attributes()
                    .map(|pair| (pair.key.to_string(), pair.value.to_string()))
                    .collect(),
                count: point.count(),
                sum: point.sum().to_string(),
            });
        }
    }
    recorded
}

fn record(extra: &str, response: Option<&str>) -> Vec<Recorded> {
    record_with(&OpenTelemetryConfig::default(), None, extra, response)
}

fn sums(recorded: &[Recorded], name: &str) -> Vec<String> {
    let mut found: Vec<String> = recorded
        .iter()
        .filter(|entry| entry.name == name)
        .map(|entry| entry.sum.clone())
        .collect();
    found.sort();
    found
}

fn duration_attributes(recorded: &[Recorded]) -> &BTreeMap<String, String> {
    &recorded
        .iter()
        .find(|entry| entry.name == DURATION)
        .expect("duration")
        .attributes
}

#[test]
fn a_streaming_call_records_every_histogram_once() {
    let extra = r#"{
        "optional_params": {"stream": true},
        "api_call_start_time": 10.5,
        "completion_start_time": 11.0,
        "response_cost": 0.5
    }"#;
    let recorded = record(extra, Some(USAGE_2_4));
    assert_eq!(sums(&recorded, DURATION), ["3"]);
    assert_eq!(sums(&recorded, TOKENS), ["2", "4"]);
    assert_eq!(sums(&recorded, COST), ["0.5"]);
    assert_eq!(sums(&recorded, TTFT), ["0.5"]);
    assert_eq!(sums(&recorded, TPOT), ["0.5"]);
    assert_eq!(sums(&recorded, RESPONSE), ["2.5"]);
    assert!(recorded.iter().all(|entry| entry.count == 1));
}

#[test]
fn the_common_attributes_name_the_call_provider_model_and_metadata() {
    let recorded = record("{}", None);
    let expected: BTreeMap<String, String> = [
        ("gen_ai.operation.name", "chat"),
        ("gen_ai.system", "openai"),
        ("gen_ai.request.model", "gpt-4o"),
        ("gen_ai.framework", "litellm"),
        ("metadata.user_api_key_alias", "key-a"),
        ("hidden_params", r#"{"api_base":"https://x"}"#),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    assert_eq!(duration_attributes(&recorded), &expected);
}

#[test]
fn metadata_values_that_are_not_text_are_written_as_json() {
    let extra = r#"{"standard_logging_object": {"metadata": {
        "requester_metadata": {"team": 1},
        "applied_guardrails": ["a", "b"],
        "requester_ip_address": true,
        "user_api_key_org_id": null,
        "user_api_key_user_id": ""
    }}}"#;
    let recorded = record(extra, None);
    let attributes = duration_attributes(&recorded);
    assert_eq!(attributes["metadata.requester_metadata"], r#"{"team":1}"#);
    assert_eq!(attributes["metadata.applied_guardrails"], r#"["a","b"]"#);
    assert_eq!(attributes["metadata.requester_ip_address"], "true");
    assert_eq!(attributes["metadata.user_api_key_user_id"], "");
    assert!(!attributes.contains_key("metadata.user_api_key_org_id"));
    assert!(!attributes.contains_key("hidden_params"));
}

#[test]
fn an_empty_hidden_params_object_is_not_an_attribute() {
    let recorded = record(
        r#"{"standard_logging_object": {"hidden_params": {}}}"#,
        None,
    );
    assert!(!duration_attributes(&recorded).contains_key("hidden_params"));
}

#[test]
fn a_call_without_a_provider_has_no_system_attribute() {
    let recorded = record(r#"{"litellm_params": {"custom_llm_provider": ""}}"#, None);
    assert!(!duration_attributes(&recorded).contains_key("gen_ai.system"));
}

#[rstest]
#[case::embedding_with_opt_in(true, "aembedding", "embeddings")]
#[case::text_completion_with_opt_in(true, "atext_completion", "text_completion")]
#[case::chat_with_opt_in(true, "acompletion", "chat")]
#[case::embedding_without_opt_in(false, "aembedding", "chat")]
fn the_operation_name_follows_the_call_type_only_with_the_semconv_opt_in(
    #[case] opt_in: bool,
    #[case] call_type: &str,
    #[case] expected: &str,
) {
    let mut config = base_config();
    if opt_in {
        config
            .semconv_stability_opt_in
            .insert(OTELSemconvCategory::GenAiLatestExperimental);
    }
    let extra = format!(r#"{{"call_type": "{call_type}"}}"#);
    let recorded = record_with(&config, None, &extra, None);
    assert_eq!(
        duration_attributes(&recorded)["gen_ai.operation.name"],
        expected
    );
}

#[test]
fn token_usage_carries_the_token_type_and_the_other_attributes() {
    let recorded = record("{}", Some(USAGE_2_4));
    let tokens: Vec<&Recorded> = recorded
        .iter()
        .filter(|entry| entry.name == TOKENS)
        .collect();
    let by_type: BTreeMap<&str, &str> = tokens
        .iter()
        .map(|entry| {
            (
                entry.attributes["gen_ai.token.type"].as_str(),
                entry.sum.as_str(),
            )
        })
        .collect();
    assert_eq!(by_type, BTreeMap::from([("input", "2"), ("output", "4")]));
    assert!(
        tokens
            .iter()
            .all(|entry| entry.attributes["gen_ai.system"] == "openai")
    );
}

#[rstest]
#[case::absent_usage(r#"{"id": "x"}"#)]
#[case::empty_usage(r#"{"usage": {}}"#)]
fn a_response_without_usage_records_no_token_usage(#[case] response: &str) {
    let recorded = record("{}", Some(response));
    assert!(sums(&recorded, TOKENS).is_empty());
    assert_eq!(sums(&recorded, DURATION), ["3"]);
}

#[test]
fn missing_token_counts_record_zero() {
    let recorded = record("{}", Some(r#"{"usage": {"total_tokens": 9}}"#));
    assert_eq!(sums(&recorded, TOKENS), ["0", "0"]);
}

#[rstest]
#[case::get_responses("get_responses", "", "", false)]
#[case::vector_store_list("avector_store_list", "", "", false)]
#[case::background_response("get_responses", r#", "background": true"#, "", true)]
#[case::background_false("get_responses", r#", "background": false"#, "", false)]
#[case::cost_poll_metadata(
    "get_responses",
    "",
    r#", "metadata": {"internal_call_origin": "background_response_cost_poll"}"#,
    true
)]
#[case::cost_poll_litellm_metadata(
    "get_responses",
    "",
    r#", "litellm_metadata": {"internal_call_origin": "background_response_cost_poll"}"#,
    true
)]
#[case::other_origin(
    "get_responses",
    "",
    r#", "metadata": {"internal_call_origin": "shadow_eval_judge"}"#,
    false
)]
#[case::metadata_wins_over_litellm_metadata(
    "get_responses",
    "",
    r#", "metadata": {"internal_call_origin": "shadow_eval_judge"}, "litellm_metadata": {"internal_call_origin": "background_response_cost_poll"}"#,
    false
)]
#[case::inference_call("acompletion", r#", "background": false"#, "", true)]
fn unbilled_calls_skip_token_usage_and_time_per_output_token(
    #[case] call_type: &str,
    #[case] response_fields: &str,
    #[case] params_fields: &str,
    #[case] billed: bool,
) {
    let response =
        format!(r#"{{"usage": {{"prompt_tokens": 2, "completion_tokens": 4}}{response_fields}}}"#);
    let extra = format!(
        r#"{{"call_type": "{call_type}", "api_call_start_time": 10.5,
        "litellm_params": {{"custom_llm_provider": "openai"{params_fields}}}}}"#
    );
    let recorded = record(&extra, Some(&response));
    assert_eq!(!sums(&recorded, TOKENS).is_empty(), billed);
    assert_eq!(!sums(&recorded, TPOT).is_empty(), billed);
    assert_eq!(sums(&recorded, DURATION), ["3"]);
    assert_eq!(sums(&recorded, RESPONSE), ["2.5"]);
}

#[rstest]
#[case::absent("{}", false)]
#[case::zero(r#"{"response_cost": 0}"#, false)]
#[case::positive(r#"{"response_cost": 0.25}"#, true)]
fn cost_is_recorded_only_when_it_is_non_zero(#[case] extra: &str, #[case] recorded_cost: bool) {
    let recorded = record(extra, None);
    assert_eq!(!sums(&recorded, COST).is_empty(), recorded_cost);
}

#[rstest]
#[case::streaming(r#"{"optional_params": {"stream": true}, "api_call_start_time": 10.5, "completion_start_time": 11.25}"#, &["0.75"])]
#[case::not_streaming(r#"{"optional_params": {"stream": false}, "api_call_start_time": 10.5, "completion_start_time": 11.25}"#, &[])]
#[case::stream_missing(r#"{"api_call_start_time": 10.5, "completion_start_time": 11.25}"#, &[])]
#[case::no_first_token(r#"{"optional_params": {"stream": true}, "api_call_start_time": 10.5}"#, &[])]
#[case::no_api_start(r#"{"optional_params": {"stream": true}, "completion_start_time": 11.25}"#, &[])]
#[case::unparsed_text(r#"{"optional_params": {"stream": true}, "api_call_start_time": "not a time", "completion_start_time": 11.25}"#, &[])]
fn time_to_first_token_measures_streaming_calls(#[case] extra: &str, #[case] expected: &[&str]) {
    let recorded = record(extra, None);
    assert_eq!(sums(&recorded, TTFT), expected);
}

#[rstest]
#[case::streaming_uses_first_token(
    r#"{"api_call_start_time": 10.5, "completion_start_time": 11}"#,
    "0.5"
)]
#[case::non_streaming_uses_api_start(r#"{"api_call_start_time": 11}"#, "0.5")]
#[case::neither_uses_the_whole_duration("{}", "0.75")]
#[case::unparsed_first_token_uses_duration(
    r#"{"completion_start_time": "later", "api_call_start_time": 11}"#,
    "0.75"
)]
#[case::unparsed_api_start_uses_duration(r#"{"api_call_start_time": "later"}"#, "0.75")]
fn time_per_output_token_divides_the_generation_time_by_completion_tokens(
    #[case] extra: &str,
    #[case] expected: &str,
) {
    let recorded = record(extra, Some(USAGE_1_4));
    assert_eq!(sums(&recorded, TPOT), [expected]);
}

#[rstest]
#[case::zero_tokens(r#"{"usage": {"completion_tokens": 0}}"#)]
#[case::no_tokens(r#"{"usage": {"prompt_tokens": 3}}"#)]
#[case::no_usage(r#"{"id": "x"}"#)]
fn time_per_output_token_needs_positive_completion_tokens(#[case] response: &str) {
    let recorded = record(r#"{"api_call_start_time": 11}"#, Some(response));
    assert!(sums(&recorded, TPOT).is_empty());
}

#[test]
fn time_per_output_token_skips_a_generation_time_that_is_not_positive() {
    let extra = r#"{"api_call_start_time": 11, "completion_start_time": 13}"#;
    let recorded = record(extra, Some(USAGE_1_4));
    assert!(sums(&recorded, TPOT).is_empty());
}

#[test]
fn time_per_output_token_is_skipped_without_a_response() {
    let recorded = record(r#"{"api_call_start_time": 11}"#, None);
    assert!(sums(&recorded, TPOT).is_empty());
}

#[rstest]
#[case::end_from_the_call(r#"{"api_call_start_time": 10.5}"#, &["2.5"])]
#[case::end_from_the_kwargs(r#"{"api_call_start_time": 10.5, "end_time": 12}"#, &["1.5"])]
#[case::no_api_start("{}", &[])]
#[case::unparsed_api_start(r#"{"api_call_start_time": "soon"}"#, &[])]
#[case::unparsed_end(r#"{"api_call_start_time": 10.5, "end_time": "soon"}"#, &[])]
#[case::zero_length(r#"{"api_call_start_time": 13}"#, &[])]
#[case::negative_length(r#"{"api_call_start_time": 14}"#, &[])]
fn response_duration_runs_from_the_api_call_start_to_the_end(
    #[case] extra: &str,
    #[case] expected: &[&str],
) {
    let recorded = record(extra, None);
    assert_eq!(sums(&recorded, RESPONSE), expected);
}

#[rstest]
#[case::with_microseconds("1970-01-01 00:00:10.500000", "2.5")]
#[case::without_microseconds("1970-01-01 00:00:10", "3")]
fn timestamp_text_is_read_in_the_configured_local_offset(
    #[case] text: &str,
    #[case] expected: &str,
) {
    let recorded = record(&format!(r#"{{"api_call_start_time": "{text}"}}"#), None);
    assert_eq!(sums(&recorded, RESPONSE), [expected]);
}

#[test]
fn a_timestamp_of_the_wrong_type_is_an_error_naming_the_field() {
    let exporter = InMemoryMetricExporter::default();
    let (recorder, _provider) = build_recorder(&OpenTelemetryConfig::default(), None, &exporter);
    let call_kwargs = kwargs(r#"{"api_call_start_time": {"seconds": 1}}"#);
    let error = recorder
        .record_metrics(&call_of(&call_kwargs, None))
        .expect_err("wrong type");
    assert!(error.to_string().contains("api_call_start_time"));
}

#[test]
fn a_filter_in_the_config_limits_the_recorded_attributes_but_never_the_token_type() {
    let mut config = base_config();
    config.attributes = Some(typed(r#"{"include_list": ["gen_ai.system"]}"#));
    let recorded = record_with(&config, None, "{}", Some(USAGE_2_4));
    let duration: BTreeMap<String, String> =
        BTreeMap::from([("gen_ai.system".to_owned(), "openai".to_owned())]);
    assert_eq!(duration_attributes(&recorded), &duration);
    let token_keys: Vec<Vec<&str>> = recorded
        .iter()
        .filter(|entry| entry.name == TOKENS)
        .map(|entry| entry.attributes.keys().map(String::as_str).collect())
        .collect();
    assert_eq!(token_keys, [["gen_ai.system", "gen_ai.token.type"]; 2]);
}

#[test]
fn an_exclude_list_removes_only_the_named_attributes() {
    let mut config = base_config();
    config.attributes = Some(typed(
        r#"{"exclude_list": ["hidden_params", "metadata.user_api_key_alias"]}"#,
    ));
    let recorded = record_with(&config, None, "{}", None);
    let keys: Vec<&str> = duration_attributes(&recorded)
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "gen_ai.framework",
            "gen_ai.operation.name",
            "gen_ai.request.model",
            "gen_ai.system"
        ]
    );
}

#[test]
fn a_bad_filter_in_the_config_fails_the_recorder_construction() {
    let mut config = base_config();
    config.attributes = Some(typed(r#"{"include_list": ["gen_ai.token.type"]}"#));
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter).build())
        .build();
    let result = LegacyMetricRecorder::new(
        create_instruments(&provider.meter("legacy-metrics-test")),
        &config,
        None,
        Arc::new(NoSettings),
        FixedOffset::east_opt(0).expect("utc"),
    );
    assert!(result.is_err());
}

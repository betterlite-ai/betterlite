use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_logging::otel::legacy_metrics::filter::CallbackSettingsView;
use rstest::rstest;

use super::support::{
    DURATION, TOKENS, USAGE_2_4, attributes_of, build, call_of, flush, kwargs, late_settings,
    record_with,
};

#[test]
fn an_include_list_keeps_only_the_named_attributes_and_the_token_type() {
    let settings = late_settings(Some(r#"{"include_list": ["gen_ai.system"]}"#));
    let recorded = record_with(settings, None, "{}", Some(USAGE_2_4));
    let expected = BTreeMap::from([("gen_ai.system".to_owned(), "openai".to_owned())]);
    assert_eq!(attributes_of(&recorded, DURATION), expected);
    let token_keys: Vec<Vec<&str>> = recorded
        .iter()
        .filter(|entry| entry.name == TOKENS)
        .map(|entry| entry.attributes.keys().map(String::as_str).collect())
        .collect();
    assert_eq!(token_keys, [["gen_ai.system", "gen_ai.token.type"]; 2]);
}

#[test]
fn an_exclude_list_removes_only_the_named_attributes() {
    let settings = late_settings(Some(
        r#"{"exclude_list": ["hidden_params", "metadata.user_api_key_alias"]}"#,
    ));
    let recorded = record_with(settings, None, "{}", None);
    let attributes = attributes_of(&recorded, DURATION);
    let keys: Vec<&str> = attributes.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "gen_ai.framework",
            "gen_ai.operation.name",
            "gen_ai.provider.name",
            "gen_ai.request.model",
            "gen_ai.system"
        ]
    );
}

#[rstest]
#[case::empty_include(r#"{"include_list": []}"#)]
#[case::empty_exclude(r#"{"exclude_list": []}"#)]
fn an_empty_list_makes_no_filter(#[case] filter: &str) {
    let settings = late_settings(Some(filter));
    let recorded = record_with(settings, None, "{}", None);
    assert!(attributes_of(&recorded, DURATION).contains_key("gen_ai.framework"));
}

#[test]
fn the_filter_is_read_on_the_first_record_not_at_construction() {
    let settings = late_settings(None);
    let cloned = Arc::clone(&settings);
    let view: Arc<dyn CallbackSettingsView> = cloned;
    let (recorder, provider, exporter) = build(view, None);
    settings.set(Some(r#"{"include_list": ["gen_ai.system"]}"#));
    let call_kwargs = kwargs("{}");
    recorder
        .record(&call_of(&call_kwargs, None))
        .expect("record");
    let points = flush(&provider, &exporter);
    let expected = BTreeMap::from([("gen_ai.system".to_owned(), "openai".to_owned())]);
    assert_eq!(attributes_of(&points, DURATION), expected);
}

#[test]
fn a_bad_filter_raises_on_the_first_record_and_a_corrected_one_is_used_afterwards() {
    let settings = late_settings(Some(
        r#"{"include_list": ["gen_ai.system"], "exclude_list": ["gen_ai.framework"]}"#,
    ));
    let cloned = Arc::clone(&settings);
    let view: Arc<dyn CallbackSettingsView> = cloned;
    let (recorder, provider, exporter) = build(view, None);
    let call_kwargs = kwargs("{}");
    let call = call_of(&call_kwargs, None);
    let error = recorder.record(&call).expect_err("both lists");
    assert!(
        error
            .to_string()
            .contains("include_list and exclude_list are mutually exclusive")
    );
    settings.set(Some(r#"{"include_list": ["gen_ai.system"]}"#));
    recorder.record(&call).expect("corrected filter");
    let points = flush(&provider, &exporter);
    let expected = BTreeMap::from([("gen_ai.system".to_owned(), "openai".to_owned())]);
    assert_eq!(attributes_of(&points, DURATION), expected);
}

#[test]
fn a_filter_naming_an_unknown_attribute_is_an_error() {
    let settings = late_settings(Some(r#"{"include_list": ["user.id"]}"#));
    let (recorder, _provider, _exporter) = build(settings, None);
    let call_kwargs = kwargs("{}");
    let error = recorder
        .record(&call_of(&call_kwargs, None))
        .expect_err("unknown name");
    assert!(error.to_string().contains("user.id"));
}

#[test]
fn a_filter_that_names_the_token_type_is_rejected() {
    let settings = late_settings(Some(r#"{"include_list": ["gen_ai.token.type"]}"#));
    let (recorder, _provider, _exporter) = build(settings, None);
    let call_kwargs = kwargs("{}");
    let error = recorder
        .record(&call_of(&call_kwargs, None))
        .expect_err("token type");
    assert!(error.to_string().contains("gen_ai.token.type"));
}

#[test]
fn another_callback_name_ignores_the_otel_filter() {
    let settings = late_settings(Some(r#"{"include_list": ["gen_ai.system"]}"#));
    let recorded = record_with(settings, Some("datadog"), "{}", None);
    let attributes = attributes_of(&recorded, DURATION);
    assert_eq!(attributes.len(), 7);
    assert!(attributes.contains_key("gen_ai.framework"));
    assert!(attributes.contains_key("metadata.user_api_key_alias"));
}

#[test]
fn the_otel_callback_name_reads_the_filter() {
    let settings = late_settings(Some(r#"{"include_list": ["gen_ai.system"]}"#));
    let recorded = record_with(settings, Some("otel"), "{}", None);
    let expected = BTreeMap::from([("gen_ai.system".to_owned(), "openai".to_owned())]);
    assert_eq!(attributes_of(&recorded, DURATION), expected);
}

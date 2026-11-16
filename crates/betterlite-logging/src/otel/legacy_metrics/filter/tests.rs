#![cfg(test)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rstest::rstest;

use super::{
    CallbackSettingsView, METRIC_METADATA_KEYS, MetricAttributeFilter, MetricFilterState,
    build_metric_attribute_filter, resolve_metric_filter,
};
use crate::otel::legacy_metrics::error::LegacyMetricsError;
use crate::otel::model::utils::coercion::OtelModelValue;

struct CountingSettings {
    attributes: Option<OtelModelValue>,
    reads: AtomicUsize,
}

impl CountingSettings {
    fn new(attributes: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            attributes: attributes.map(typed),
            reads: AtomicUsize::new(0),
        })
    }
}

impl CallbackSettingsView for CountingSettings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.attributes.clone()
    }
}

fn typed(text: &str) -> OtelModelValue {
    serde_json::from_str(text).expect("typed value")
}

fn names(listed: &[&str]) -> Vec<String> {
    listed.iter().map(|name| (*name).to_owned()).collect()
}

fn set(listed: &[&str]) -> BTreeSet<String> {
    listed.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn the_metric_metadata_keys_are_the_fifteen_litellm_names() {
    assert_eq!(METRIC_METADATA_KEYS.len(), 15);
    assert_eq!(METRIC_METADATA_KEYS.first(), Some(&"user_api_key_hash"));
    assert_eq!(
        METRIC_METADATA_KEYS.last(),
        Some(&"vector_store_request_metadata")
    );
}

#[test]
fn every_base_name_and_every_metadata_variant_is_valid() {
    let mut valid = names(&[
        "gen_ai.operation.name",
        "gen_ai.provider.name",
        "gen_ai.system",
        "gen_ai.request.model",
        "gen_ai.framework",
        "hidden_params",
    ]);
    valid.extend(
        METRIC_METADATA_KEYS
            .iter()
            .map(|key| format!("metadata.{key}")),
    );
    let filter = MetricAttributeFilter {
        include_list: Some(valid.clone()),
        exclude_list: None,
    };
    let resolved = resolve_metric_filter(Some(&filter)).expect("all names are valid");
    assert_eq!(resolved.include, Some(valid.into_iter().collect()));
}

#[rstest]
#[case::dict(r#"{"include_list": ["gen_ai.system"]}"#, Some(names(&["gen_ai.system"])), None)]
#[case::exclude(r#"{"exclude_list": ["hidden_params"]}"#, None, Some(names(&["hidden_params"])))]
#[case::empty(r#"{}"#, None, None)]
#[case::null_lists(r#"{"include_list": null, "exclude_list": null}"#, None, None)]
#[case::extra_keys_are_ignored(r#"{"other": 1, "include_list": []}"#, Some(vec![]), None)]
fn a_mapping_builds_a_filter(
    #[case] value: &str,
    #[case] include: Option<Vec<String>>,
    #[case] exclude: Option<Vec<String>>,
) {
    let built = build_metric_attribute_filter(&typed(value)).expect("built");
    assert_eq!(
        built,
        MetricAttributeFilter {
            include_list: include,
            exclude_list: exclude,
        }
    );
}

#[rstest]
#[case::text(r#""gen_ai.system""#)]
#[case::list(r#"["gen_ai.system"]"#)]
#[case::number(r#"3"#)]
#[case::bool(r#"true"#)]
fn a_value_that_is_not_a_mapping_is_rejected(#[case] value: &str) {
    let error = build_metric_attribute_filter(&typed(value)).expect_err("not a mapping");
    assert!(matches!(error, LegacyMetricsError::AttributesNotMapping));
}

#[rstest]
#[case::text_list(r#"{"include_list": "gen_ai.system"}"#)]
#[case::number_items(r#"{"exclude_list": [1]}"#)]
fn a_list_of_the_wrong_type_is_an_error_naming_the_shape(#[case] value: &str) {
    let error = build_metric_attribute_filter(&typed(value)).expect_err("wrong type");
    assert!(matches!(error, LegacyMetricsError::AttributesShape { .. }));
}

#[test]
fn no_filter_resolves_to_nothing() {
    let resolved = resolve_metric_filter(None).expect("resolved");
    assert_eq!((resolved.include, resolved.exclude), (None, None));
}

#[test]
fn empty_lists_resolve_to_nothing() {
    let filter = MetricAttributeFilter {
        include_list: Some(vec![]),
        exclude_list: Some(vec![]),
    };
    let resolved = resolve_metric_filter(Some(&filter)).expect("resolved");
    assert_eq!((resolved.include, resolved.exclude), (None, None));
}

#[test]
fn one_list_resolves_to_a_set_and_the_other_stays_empty() {
    let include = MetricAttributeFilter {
        include_list: Some(names(&["gen_ai.system", "gen_ai.system"])),
        exclude_list: Some(vec![]),
    };
    let from_include = resolve_metric_filter(Some(&include)).expect("include");
    assert_eq!(from_include.include, Some(set(&["gen_ai.system"])));
    assert_eq!(from_include.exclude, None);
    let exclude = MetricAttributeFilter {
        include_list: None,
        exclude_list: Some(names(&["hidden_params"])),
    };
    let from_exclude = resolve_metric_filter(Some(&exclude)).expect("exclude");
    assert_eq!(from_exclude.include, None);
    assert_eq!(from_exclude.exclude, Some(set(&["hidden_params"])));
}

#[test]
fn both_lists_together_are_rejected() {
    let filter = MetricAttributeFilter {
        include_list: Some(names(&["gen_ai.system"])),
        exclude_list: Some(names(&["hidden_params"])),
    };
    let error = resolve_metric_filter(Some(&filter)).expect_err("both");
    assert!(matches!(error, LegacyMetricsError::BothLists));
}

#[rstest]
#[case::include(Some(names(&["gen_ai.system", "gen_ai.token.type"])), None)]
#[case::exclude(None, Some(names(&["gen_ai.token.type"])))]
fn the_token_type_attribute_can_never_be_filtered(
    #[case] include: Option<Vec<String>>,
    #[case] exclude: Option<Vec<String>>,
) {
    let filter = MetricAttributeFilter {
        include_list: include,
        exclude_list: exclude,
    };
    let error = resolve_metric_filter(Some(&filter)).expect_err("token type");
    assert!(matches!(error, LegacyMetricsError::TokenTypeFiltered));
}

#[test]
fn unknown_names_are_listed_in_sorted_order() {
    let filter = MetricAttributeFilter {
        include_list: Some(names(&["zeta", "gen_ai.system", "metadata.nope", "alpha"])),
        exclude_list: None,
    };
    let error = resolve_metric_filter(Some(&filter)).expect_err("unknown");
    assert!(matches!(
        &error,
        LegacyMetricsError::UnknownAttributeNames { names: listed } if listed ==&["alpha", "metadata.nope", "zeta"]
    ));
}

#[rstest]
#[case::include_hit(Some(&["gen_ai.system"][..]), None, "gen_ai.system", true)]
#[case::include_miss(Some(&["gen_ai.system"][..]), None, "hidden_params", false)]
#[case::exclude_hit(None, Some(&["hidden_params"][..]), "hidden_params", false)]
#[case::exclude_miss(None, Some(&["hidden_params"][..]), "gen_ai.system", true)]
#[case::no_filter(None, None, "anything", true)]
fn the_resolved_filter_allows_by_list(
    #[case] include: Option<&[&str]>,
    #[case] exclude: Option<&[&str]>,
    #[case] key: &str,
    #[case] allowed: bool,
) {
    let filter = MetricAttributeFilter {
        include_list: include.map(names),
        exclude_list: exclude.map(names),
    };
    let resolved = resolve_metric_filter(Some(&filter)).expect("resolved");
    assert_eq!(resolved.allows(key), allowed);
}

#[test]
fn a_configured_filter_is_validated_when_the_state_is_built() {
    let settings = CountingSettings::new(None);
    let bad = typed(r#"{"include_list": ["gen_ai.token.type"]}"#);
    let result = MetricFilterState::new(Some(bad), None, Arc::clone(&settings));
    assert!(matches!(result, Err(LegacyMetricsError::TokenTypeFiltered)));
    assert_eq!(settings.reads.load(Ordering::SeqCst), 0);
    let not_mapping = MetricFilterState::new(Some(typed(r#""x""#)), None, settings);
    assert!(matches!(
        not_mapping,
        Err(LegacyMetricsError::AttributesNotMapping)
    ));
}

#[test]
fn an_unset_filter_resolves_on_the_first_record_and_only_once() {
    let settings = CountingSettings::new(Some(r#"{"exclude_list": ["hidden_params"]}"#));
    let state = MetricFilterState::new(None, None, Arc::clone(&settings)).expect("state");
    assert_eq!(settings.reads.load(Ordering::SeqCst), 0);
    assert!(!state.ensure().expect("first").allows("hidden_params"));
    assert!(state.ensure().expect("second").allows("gen_ai.system"));
    assert_eq!(settings.reads.load(Ordering::SeqCst), 1);
}

#[rstest]
#[case::no_name(None, 1, false)]
#[case::otel(Some("otel"), 1, false)]
#[case::langfuse(Some("langfuse_otel"), 0, true)]
#[case::arize(Some("arize"), 0, true)]
fn callback_settings_are_read_only_for_the_default_otel_callback(
    #[case] callback_name: Option<&str>,
    #[case] reads: usize,
    #[case] allows_hidden: bool,
) {
    let settings = CountingSettings::new(Some(r#"{"exclude_list": ["hidden_params"]}"#));
    let state = MetricFilterState::new(
        None,
        callback_name.map(str::to_owned),
        Arc::clone(&settings),
    )
    .expect("state");
    assert_eq!(
        state.ensure().expect("resolved").allows("hidden_params"),
        allows_hidden
    );
    assert_eq!(settings.reads.load(Ordering::SeqCst), reads);
}

#[test]
fn the_config_attributes_win_over_the_callback_settings() {
    let settings = CountingSettings::new(Some(r#"{"exclude_list": ["hidden_params"]}"#));
    let config = typed(r#"{"exclude_list": ["gen_ai.system"]}"#);
    let state = MetricFilterState::new(Some(config), None, Arc::clone(&settings)).expect("state");
    let resolved = state.ensure().expect("resolved");
    assert!(resolved.allows("hidden_params"));
    assert!(!resolved.allows("gen_ai.system"));
    assert_eq!(settings.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn null_config_attributes_and_null_settings_mean_unset() {
    let settings = CountingSettings::new(Some("null"));
    let state =
        MetricFilterState::new(Some(OtelModelValue::Null(())), None, settings).expect("state");
    assert!(state.ensure().expect("resolved").allows("anything"));
}

#[test]
fn a_bad_setting_fails_every_record_and_is_not_cached() {
    let settings = CountingSettings::new(Some(r#"{"include_list": ["nope"]}"#));
    let state = MetricFilterState::new(None, None, Arc::clone(&settings)).expect("state");
    assert!(state.ensure().is_err());
    assert!(state.ensure().is_err());
    assert_eq!(settings.reads.load(Ordering::SeqCst), 2);
}

#[test]
fn filtering_keeps_allowed_present_attributes_in_order() {
    let settings = CountingSettings::new(Some(
        r#"{"include_list": ["gen_ai.system", "hidden_params"]}"#,
    ));
    let state = MetricFilterState::new(None, None, settings).expect("state");
    let kept = state
        .filter_attributes(vec![
            ("gen_ai.system".to_owned(), Some("openai".to_owned())),
            ("gen_ai.request.model".to_owned(), Some("gpt".to_owned())),
            ("hidden_params".to_owned(), None),
            ("not.listed".to_owned(), Some("x".to_owned())),
        ])
        .expect("filtered");
    let pairs: Vec<(String, String)> = kept
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.to_string()))
        .collect();
    assert_eq!(pairs, [("gen_ai.system".to_owned(), "openai".to_owned())]);
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rstest::rstest;

use super::{LabelFactoryError, LabelSource, ProjectedLabels, prometheus_label_factory};
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::test_support::{
    self, CountingResolver, PROMETHEUS_ONLY, Resolution, texts,
};
use crate::prometheus_label_schema::PrometheusLabelSettings;

fn labels(
    supported: &[&str],
    values: &UserApiKeyLabelValues,
    source: LabelSource<'_>,
) -> ProjectedLabels {
    prometheus_label_factory(&texts(supported), values, source).expect("the projected labels")
}

fn context_for(
    values: &UserApiKeyLabelValues,
    settings: &PrometheusLabelSettings,
    resolution: Resolution,
) -> PrometheusLabelFactoryContext {
    PrometheusLabelFactoryContext::with_resolver(
        values,
        settings,
        Arc::new(CountingResolver::new(resolution)),
    )
    .expect("the context")
}

fn map_of(entries: &[(&str, Option<&str>)]) -> ProjectedLabels {
    entries
        .iter()
        .map(|(label, value)| ((*label).to_owned(), value.map(str::to_owned)))
        .collect()
}

#[test]
fn the_raw_path_filters_and_fills_the_supported_labels() {
    let values = test_support::values(&[("user", "u1"), ("team", "t1")]);
    let settings = test_support::settings(&[]);
    let projected = labels(
        &["user", "model"],
        &values,
        LabelSource::Values {
            settings: &settings,
            end_user_settings: EndUserCostTrackingSettings::default(),
        },
    );
    assert_eq!(projected, map_of(&[("model", None), ("user", Some("u1"))]));
}

#[test]
fn the_context_path_matches_the_raw_path() {
    let mut values = test_support::values(&[("user", "u1"), ("user_agent", "say \"hi\"")]);
    values.tags = texts(&["prod"]);
    let settings = test_support::settings(&["prod"]);
    let context = context_for(&values, &settings, Resolution::Missing);
    let supported = ["user", "user_agent", "model", "tag_prod"];
    let from_context =
        prometheus_label_factory(&texts(&supported), &values, LabelSource::Context(&context))
            .expect("the context labels");
    let from_values = prometheus_label_factory(
        &texts(&supported),
        &values,
        LabelSource::Values {
            settings: &settings,
            end_user_settings: EndUserCostTrackingSettings::default(),
        },
    )
    .expect("the raw labels");
    assert_eq!(from_context, from_values);
    assert_eq!(
        from_values,
        map_of(&[
            ("model", None),
            ("tag_prod", Some("true")),
            ("user", Some("u1")),
            ("user_agent", Some("say \\\"hi\\\"")),
        ])
    );
}

#[test]
fn a_context_from_other_values_is_rejected() {
    let settings = test_support::settings(&[]);
    let context = context_for(
        &test_support::values(&[("user", "u1")]),
        &settings,
        Resolution::Missing,
    );
    let other = test_support::values(&[("user", "u2")]);
    let result =
        prometheus_label_factory(&texts(&["user"]), &other, LabelSource::Context(&context));
    assert!(matches!(result, Err(LabelFactoryError::ContextMismatch)));
}

#[test]
fn an_equal_copy_of_the_values_is_accepted() {
    let values = test_support::values(&[("user", "u1")]);
    let settings = test_support::settings(&[]);
    let context = context_for(&values, &settings, Resolution::Missing);
    let copy = values.clone();
    let projected = labels(&["user"], &copy, LabelSource::Context(&context));
    assert_eq!(projected, map_of(&[("user", Some("u1"))]));
}

#[rstest]
#[case::exact_true(None, Some(true), Some("e1"))]
#[case::unset_flag(None, None, None)]
#[case::false_flag(None, Some(false), None)]
#[case::disabled(Some(true), Some(true), None)]
fn end_user_is_resolved_through_the_cost_tracking_policy(
    #[case] disable: Option<bool>,
    #[case] prometheus_only: Option<bool>,
    #[case] expected_end_user: Option<&str>,
) {
    let values = test_support::values(&[("end_user", "e1")]);
    let settings = test_support::settings(&[]);
    let end_user_settings = EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: disable,
        enable_prometheus_only: prometheus_only,
    };
    let projected = labels(
        &["end_user"],
        &values,
        LabelSource::Values {
            settings: &settings,
            end_user_settings,
        },
    );
    assert_eq!(
        projected.get("end_user").cloned(),
        Some(expected_end_user.map(str::to_owned))
    );
}

#[test]
fn the_raw_path_leaves_an_unsupported_end_user_out() {
    let values = test_support::values(&[("user", "u1"), ("end_user", "e1")]);
    let settings = test_support::settings(&[]);
    let projected = labels(
        &["user"],
        &values,
        LabelSource::Values {
            settings: &settings,
            end_user_settings: PROMETHEUS_ONLY,
        },
    );
    assert_eq!(projected, map_of(&[("user", Some("u1"))]));
}

#[test]
fn the_context_path_leaves_an_unsupported_end_user_unresolved() {
    let values = test_support::values(&[("user", "u1"), ("end_user", "e1")]);
    let settings = test_support::settings(&[]);
    let resolver = Arc::new(CountingResolver::new(Resolution::Failure));
    let shared_resolver = Arc::clone(&resolver);
    let context = PrometheusLabelFactoryContext::with_resolver(&values, &settings, shared_resolver)
        .expect("the context");
    let projected = labels(&["user"], &values, LabelSource::Context(&context));
    assert_eq!(projected, map_of(&[("user", Some("u1"))]));
    assert_eq!(
        *resolver
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
        0
    );
}

#[test]
fn the_raw_path_merges_sanitized_custom_metadata_labels() {
    let mut values = test_support::values(&[("user", "u1")]);
    values.custom_metadata_labels = BTreeMap::from([
        ("my.key".to_owned(), "say \"hi\"".to_owned()),
        ("ignored".to_owned(), "x".to_owned()),
    ]);
    let settings = test_support::settings(&[]);
    let projected = labels(
        &["my_key"],
        &values,
        LabelSource::Values {
            settings: &settings,
            end_user_settings: EndUserCostTrackingSettings::default(),
        },
    );
    assert_eq!(projected, map_of(&[("my_key", Some("say \\\"hi\\\""))]));
}

#[test]
fn the_context_path_merges_sanitized_custom_metadata_labels() {
    let mut values = test_support::values(&[("user", "u1")]);
    values.custom_metadata_labels =
        BTreeMap::from([("my.key".to_owned(), "say \"hi\"".to_owned())]);
    let settings = test_support::settings(&[]);
    let context = context_for(&values, &settings, Resolution::Missing);
    let projected = labels(&["my_key"], &values, LabelSource::Context(&context));
    assert_eq!(projected, map_of(&[("my_key", Some("say \\\"hi\\\""))]));
}

#[test]
fn custom_metadata_wins_over_enum_labels_and_the_resolved_end_user() {
    let mut values = test_support::values(&[("user", "enum-user"), ("end_user", "e1")]);
    values.custom_metadata_labels = BTreeMap::from([
        ("user".to_owned(), "custom-user".to_owned()),
        ("end_user".to_owned(), "custom-end-user".to_owned()),
    ]);
    let settings = test_support::settings(&[]);
    let supported = ["user", "end_user"];
    let raw = prometheus_label_factory(
        &texts(&supported),
        &values,
        LabelSource::Values {
            settings: &settings,
            end_user_settings: PROMETHEUS_ONLY,
        },
    )
    .expect("the raw labels");
    let context = context_for(&values, &settings, Resolution::Echo);
    let from_context =
        prometheus_label_factory(&texts(&supported), &values, LabelSource::Context(&context))
            .expect("the context labels");
    let expected = map_of(&[
        ("end_user", Some("custom-end-user")),
        ("user", Some("custom-user")),
    ]);
    assert_eq!(raw, expected);
    assert_eq!(from_context, expected);
}

#[test]
fn tag_labels_are_merged_on_both_paths() {
    let mut values = test_support::values(&[("user", "u1")]);
    values.tags = texts(&["prod"]);
    let settings = test_support::settings(&["prod", "User-Agent: curl/*"]);
    let supported = ["user", "tag_prod", "tag_User_Agent__curl__"];
    let raw = prometheus_label_factory(
        &texts(&supported),
        &values,
        LabelSource::Values {
            settings: &settings,
            end_user_settings: EndUserCostTrackingSettings::default(),
        },
    )
    .expect("the raw labels");
    let context = context_for(&values, &settings, Resolution::Missing);
    let from_context =
        prometheus_label_factory(&texts(&supported), &values, LabelSource::Context(&context))
            .expect("the context labels");
    let expected = map_of(&[
        ("tag_User_Agent__curl__", Some("false")),
        ("tag_prod", Some("true")),
        ("user", Some("u1")),
    ]);
    assert_eq!(raw, expected);
    assert_eq!(from_context, expected);
}

#[test]
fn supported_labels_without_a_value_are_filled_with_none() {
    let values = test_support::values(&[("user", "u1")]);
    let settings = test_support::settings(&[]);
    let supported = ["user", "my_key"];
    let raw = prometheus_label_factory(
        &texts(&supported),
        &values,
        LabelSource::Values {
            settings: &settings,
            end_user_settings: EndUserCostTrackingSettings::default(),
        },
    )
    .expect("the raw labels");
    let context = context_for(&values, &settings, Resolution::Missing);
    let from_context =
        prometheus_label_factory(&texts(&supported), &values, LabelSource::Context(&context))
            .expect("the context labels");
    let expected = map_of(&[("my_key", None), ("user", Some("u1"))]);
    assert_eq!(raw, expected);
    assert_eq!(from_context, expected);
}

#[test]
fn the_context_path_resolves_the_end_user_once() {
    let values = test_support::values(&[("end_user", "e1")]);
    let settings = test_support::settings(&[]);
    let resolver = Arc::new(CountingResolver::new(Resolution::Echo));
    let shared_resolver = Arc::clone(&resolver);
    let context = PrometheusLabelFactoryContext::with_resolver(&values, &settings, shared_resolver)
        .expect("the context");
    let projected = labels(&["end_user"], &values, LabelSource::Context(&context));
    assert_eq!(projected, map_of(&[("end_user", Some("resolved-e1"))]));
    let again = labels(&["end_user"], &values, LabelSource::Context(&context));
    assert_eq!(again, map_of(&[("end_user", Some("resolved-e1"))]));
    assert_eq!(
        *resolver
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
        1
    );
}

#[test]
fn tag_labels_win_over_custom_metadata_with_the_same_name() {
    let mut values = test_support::values(&[("user", "u1")]);
    values.tags = texts(&["prod"]);
    values.custom_metadata_labels = BTreeMap::from([("tag_prod".to_owned(), "custom".to_owned())]);
    let settings = test_support::settings(&["prod"]);
    let supported = ["user", "tag_prod"];
    let raw = prometheus_label_factory(
        &texts(&supported),
        &values,
        LabelSource::Values {
            settings: &settings,
            end_user_settings: EndUserCostTrackingSettings::default(),
        },
    )
    .expect("the raw labels");
    let context = context_for(&values, &settings, Resolution::Missing);
    let from_context =
        prometheus_label_factory(&texts(&supported), &values, LabelSource::Context(&context))
            .expect("the context labels");
    let expected = map_of(&[("tag_prod", Some("true")), ("user", Some("u1"))]);
    assert_eq!(raw, expected);
    assert_eq!(from_context, expected);
}

#[test]
fn tags_outside_the_supported_set_are_skipped() {
    let mut values = test_support::values(&[("user", "u1")]);
    values.tags = texts(&["prod"]);
    let settings = test_support::settings(&["prod"]);
    let raw = prometheus_label_factory(
        &texts(&["user"]),
        &values,
        LabelSource::Values {
            settings: &settings,
            end_user_settings: EndUserCostTrackingSettings::default(),
        },
    )
    .expect("the raw labels");
    let context = context_for(&values, &settings, Resolution::Missing);
    let from_context =
        prometheus_label_factory(&texts(&["user"]), &values, LabelSource::Context(&context))
            .expect("the context labels");
    assert_eq!(raw, map_of(&[("user", Some("u1"))]));
    assert_eq!(from_context, map_of(&[("user", Some("u1"))]));
}

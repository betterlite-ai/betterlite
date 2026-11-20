#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use super::PrometheusLabelFactoryContext;
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus_label_projection::end_user_caching::CachedEndUser;
use crate::prometheus_label_projection::test_support::{self, CountingResolver, Resolution};

#[test]
fn the_sanitized_enum_carries_every_scalar() {
    let values = test_support::values(&[("user", "say \"hi\""), ("user_agent", "a\u{2028}b")]);
    let context = PrometheusLabelFactoryContext::new(
        &values,
        &test_support::settings(&[]),
        EndUserCostTrackingSettings::default(),
    )
    .expect("the context");
    assert_eq!(
        context.sanitized_enum.get("user").cloned(),
        Some(Some("say \\\"hi\\\"".to_owned()))
    );
    assert_eq!(
        context.sanitized_enum.get("user_agent").cloned(),
        Some(Some("ab".to_owned()))
    );
    assert_eq!(context.sanitized_enum.get("team").cloned(), Some(None));
}

#[test]
fn custom_metadata_labels_are_keyed_by_their_sanitized_name() {
    let mut values = test_support::values(&[("user", "u1")]);
    values.custom_metadata_labels =
        BTreeMap::from([("my.key".to_owned(), "say \"hi\"".to_owned())]);
    let context = PrometheusLabelFactoryContext::new(
        &values,
        &test_support::settings(&[]),
        EndUserCostTrackingSettings::default(),
    )
    .expect("the context");
    assert_eq!(
        context.custom_by_sanitized_key.get("my_key").cloned(),
        Some(Some("say \\\"hi\\\"".to_owned()))
    );
}

#[test]
fn tag_labels_come_from_the_configured_tags() {
    let mut values = test_support::values(&[("user", "u1")]);
    values.tags = test_support::texts(&["prod"]);
    let context = PrometheusLabelFactoryContext::new(
        &values,
        &test_support::settings(&["prod", "User-Agent: curl/*"]),
        EndUserCostTrackingSettings::default(),
    )
    .expect("the context");
    assert_eq!(
        context.tag_labels.get("tag_prod").cloned(),
        Some(Some("true".to_owned()))
    );
    assert_eq!(
        context.tag_labels.get("tag_User_Agent__curl__").cloned(),
        Some(Some("false".to_owned()))
    );
}

#[test]
fn settings_without_custom_tags_produce_no_tag_labels() {
    let mut values = test_support::values(&[("user", "u1")]);
    values.tags = test_support::texts(&["prod"]);
    let context = PrometheusLabelFactoryContext::new(
        &values,
        &test_support::settings(&[]),
        EndUserCostTrackingSettings::default(),
    )
    .expect("the context");
    assert!(context.tag_labels.is_empty());
}

#[test]
fn construction_does_not_resolve_the_end_user() {
    let values = test_support::values(&[("end_user", "abc")]);
    let resolver = Arc::new(CountingResolver::new(Resolution::Echo));
    let shared_resolver = Arc::clone(&resolver);
    let context = PrometheusLabelFactoryContext::with_resolver(
        &values,
        &test_support::settings(&[]),
        shared_resolver,
    )
    .expect("the context");
    assert_eq!(
        *resolver
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
        0
    );
    assert_eq!(
        *context
            .resolved_end_user
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
        CachedEndUser::NotComputed
    );
    assert_eq!(context.enum_values, values);
}

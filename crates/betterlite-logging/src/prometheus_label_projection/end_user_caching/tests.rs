#![cfg(test)]

use std::sync::{Arc, PoisonError};

use super::CachedEndUser;
use crate::end_user::EndUserIdError;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::test_support::{self, CountingResolver, Resolution};

fn context(
    resolver: &Arc<CountingResolver>,
    end_user: Option<&str>,
) -> PrometheusLabelFactoryContext {
    let mut values = test_support::values(&[]);
    values.end_user = end_user.map(str::to_owned);
    let shared_resolver = Arc::clone(resolver);
    PrometheusLabelFactoryContext::with_resolver(
        &values,
        &test_support::settings(&[]),
        shared_resolver,
    )
    .expect("the context")
}

fn calls(resolver: &CountingResolver) -> usize {
    *resolver
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

#[test]
fn the_starting_state_is_not_computed() {
    assert_eq!(CachedEndUser::default(), CachedEndUser::NotComputed);
}

#[test]
fn a_resolved_none_differs_from_the_starting_state() {
    assert_ne!(CachedEndUser::Computed(None), CachedEndUser::NotComputed);
}

#[test]
fn a_resolved_value_is_cached() {
    let resolver = Arc::new(CountingResolver::new(Resolution::Echo));
    let context = context(&resolver, Some("abc"));
    assert_eq!(
        context.get_resolved_end_user().expect("the end user"),
        Some("resolved-abc".to_owned())
    );
    assert_eq!(
        context.get_resolved_end_user().expect("the end user"),
        Some("resolved-abc".to_owned())
    );
    assert_eq!(calls(&resolver), 1);
}

#[test]
fn a_resolved_none_is_cached_too() {
    let resolver = Arc::new(CountingResolver::new(Resolution::Missing));
    let context = context(&resolver, Some("abc"));
    assert_eq!(context.get_resolved_end_user().expect("the end user"), None);
    assert_eq!(context.get_resolved_end_user().expect("the end user"), None);
    assert_eq!(calls(&resolver), 1);
}

#[test]
fn a_failed_resolution_is_retried() {
    let resolver = Arc::new(CountingResolver::new(Resolution::Failure));
    let context = context(&resolver, Some("abc"));
    assert!(matches!(
        context.get_resolved_end_user(),
        Err(EndUserIdError::Metadata(_))
    ));
    assert!(matches!(
        context.get_resolved_end_user(),
        Err(EndUserIdError::Metadata(_))
    ));
    assert_eq!(calls(&resolver), 2);
}

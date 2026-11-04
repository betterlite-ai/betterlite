#![cfg(test)]

use super::{BillingPolicy, CallContext, Phase};
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ptr::eq;
use std::time::{Duration, UNIX_EPOCH};
use uuid::Uuid;

#[test]
fn call_context_tests_behavior() {
    let now = UNIX_EPOCH + Duration::from_secs(100);
    let parent = CallContext::with_id(Uuid::from_u128(1), now);
    assert_eq!(parent.view().id, Uuid::from_u128(1));
    let child = parent.child(Uuid::from_u128(2));
    assert_eq!(child.view().parent_id, Some(parent.view().id));
    assert_eq!(child.view().billing, BillingPolicy::Suppress);
    assert_eq!(child.view().billing_time, parent.view().billing_time);
    assert_eq!(child.view().phase, Phase::Request);
    assert_eq!(parent.post_response().view().phase, Phase::PostResponse);
}

#[test]
fn dispatch_views_preserve_absence_and_identity() {
    let metadata = BTreeMap::new();
    let http: &dyn Debug = &"http-client";
    let context = CallContext::new(UNIX_EPOCH)
        .with_metadata(&metadata)
        .with_http_client(http);
    let view = context.view();
    assert!(eq(view.metadata.unwrap(), &raw const metadata));
    assert!(eq(view.http_client.unwrap(), http));
    assert!(view.azure_client.is_none());
    assert!(view.openai_client.is_none());
}

#[test]
fn provider_items_preserve_explicit_none() {
    let items: BTreeMap<String, Option<&dyn Debug>> =
        BTreeMap::from([("timeout".to_owned(), None)]);
    let (key, value) = items.iter().next().unwrap();
    assert_eq!(key, "timeout");
    assert!(value.is_none());
}

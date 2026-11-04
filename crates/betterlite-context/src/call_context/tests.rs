#![cfg(test)]

use super::{BillingPolicy, CallContext, Phase};
use std::time::{Duration, UNIX_EPOCH};
use uuid::Uuid;

#[test]
fn call_context_tests_behavior() {
    let now = UNIX_EPOCH + Duration::from_secs(100);
    let parent = CallContext::with_id(Uuid::from_u128(1), now);
    let child = parent.child(Uuid::from_u128(2));
    assert_eq!(child.parent_id, Some(parent.id));
    assert_eq!(child.billing_policy(), BillingPolicy::Suppress);
    assert_eq!(child.billing_time, parent.billing_time);
    assert_eq!(child.phase, Phase::Request);
    assert_eq!(parent.post_response().phase, Phase::PostResponse);
}

#[test]
fn dispatch_views_preserve_absence_and_identity() {
    let metadata = std::collections::BTreeMap::new();
    let http = serde_json::json!({"marker":true});
    let context = CallContext::new(UNIX_EPOCH)
        .with_metadata(&metadata)
        .with_http_client(&http);
    assert!(std::ptr::eq(context.metadata.unwrap(), &metadata));
    assert!(std::ptr::eq(context.http_client.unwrap(), &http));
    assert!(context.azure_client.is_none());
    assert!(context.openai_client.is_none());
}

#[test]
fn provider_items_preserve_explicit_none() {
    let items: std::collections::BTreeMap<String, Option<serde_json::Value>> =
        std::collections::BTreeMap::from([("timeout".to_owned(), None)]);
    let (key, value) = items.iter().next().unwrap();
    assert_eq!(key, "timeout");
    assert!(value.is_none());
}

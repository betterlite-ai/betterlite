#![cfg(test)]

use super::{BillingPolicy, CallContext, CallInputs, Phase};
use rstest::rstest;
use std::collections::BTreeMap;
use std::ptr::eq;
use std::time::{Duration, UNIX_EPOCH};
use uuid::Uuid;

#[rstest]
#[case::external_defaults(false)]
#[case::private_request(true)]
fn call_context_tests_behavior(#[case] privacy: bool) {
    let now = UNIX_EPOCH.checked_add(Duration::from_secs(100)).unwrap();
    let deadline = now.checked_add(Duration::from_secs(10)).unwrap();
    let parent = CallContext::start(CallInputs {
        id: || Uuid::from_u128(1),
        now: || now,
        deadline: Some(deadline),
        privacy,
    });
    let child = parent.child(Uuid::from_u128(2));
    let nested = child.post_response().child(Uuid::from_u128(3));
    assert_eq!(parent.view().state.id, Uuid::from_u128(1));
    assert_eq!(parent.view().state.parent_id, None);
    assert_eq!(parent.view().state.billing, BillingPolicy::Include);
    assert_eq!(parent.view().state.phase, Phase::Request);
    assert_eq!(child.view().state.id, Uuid::from_u128(2));
    assert_eq!(child.view().state.parent_id, Some(parent.view().state.id));
    assert_eq!(child.view().state.billing, BillingPolicy::Suppress);
    assert_eq!(child.view().state.billing_time, now);
    assert_eq!(child.view().state.deadline, Some(deadline));
    assert_eq!(child.view().state.privacy, privacy);
    assert_eq!(child.view().state.phase, Phase::Request);
    assert_eq!(nested.view().state.phase, Phase::PostResponse);
    assert_eq!(nested.view().state.parent_id, Some(child.view().state.id));
    assert_eq!(
        parent.post_response().view().state.billing,
        BillingPolicy::Include
    );
}

#[test]
fn dispatch_views_preserve_absence_and_identity() {
    let metadata = [("is_internal_call", true)];
    let client = String::from("client");
    let context = CallContext::start(CallInputs {
        id: || Uuid::from_u128(4),
        now: || UNIX_EPOCH,
        deadline: None,
        privacy: false,
    });
    assert!(context.view().metadata.is_none());
    assert!(context.view().client.is_none());
    let attached = context.with_dispatch(Some(&metadata), Some(&client));
    let view = attached.view();
    assert!(eq(view.metadata.unwrap(), &raw const metadata));
    assert!(eq(view.client.unwrap(), &raw const client));
    assert_eq!(view.state.billing, BillingPolicy::Include);
    assert_eq!(view.state.deadline, None);
}

#[test]
fn provider_items_preserve_explicit_none() {
    let values = BTreeMap::from([("z", None), ("a", Some(7_i32))]);
    let context = CallContext::start(CallInputs {
        id: || Uuid::from_u128(4),
        now: || UNIX_EPOCH,
        deadline: None,
        privacy: false,
    })
    .with_dispatch(Some(&values), None::<&()>);
    let stored = context.view().metadata.unwrap();
    assert!(eq(stored, &raw const values));
    assert_eq!(stored.first_key_value(), Some((&"a", &Some(7_i32))));
    assert_eq!(stored.last_key_value(), Some((&"z", &None)));
}

mod dependencies;
mod isolation;

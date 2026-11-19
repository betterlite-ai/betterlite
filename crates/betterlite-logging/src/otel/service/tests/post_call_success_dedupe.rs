use super::fixtures::{details, guardrail_entry};
use super::support::{Rig, START, call};
use crate::otel::legacy_lifecycle::lifecycle::LifecycleEvent;
use crate::otel::service::post_call::async_post_call_success_hook;

#[rstest::rstest]
#[case::hook_first(true)]
#[case::lifecycle_first(false)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(#[case] hook_first: bool) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let details = details("call-d", Some(vec![guardrail_entry()]), Some(&parent));
    if hook_first {
        async_post_call_success_hook(&lifecycle, &details, Some(&parent)).expect("success hook");
        lifecycle
            .handle(&call(&details), LifecycleEvent::Success)
            .expect("success lifecycle");
    } else {
        lifecycle
            .handle(&call(&details), LifecycleEvent::Success)
            .expect("success lifecycle");
        async_post_call_success_hook(&lifecycle, &details, Some(&parent)).expect("success hook");
    }
    assert_eq!(
        rig.named("guardrail").len(),
        1,
        "both entry points share the dedupe markers"
    );
}

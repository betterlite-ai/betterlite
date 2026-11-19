use super::fixtures::{error_config, guardrail_entry, request_data};
use super::support::Rig;
use crate::otel::service::post_call::{
    PostCallFailure, ProxyCallKey, async_post_call_failure_hook,
};

#[rstest::rstest]
#[case::without_parent()]
fn otel_legacy_service_callbacks_behavior_tests_behavior() {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let data = request_data("call-a", vec![guardrail_entry()]);
    let key = ProxyCallKey {
        parent_span: None,
        team_id: None,
        team_alias: None,
    };
    let config = error_config();
    let call = PostCallFailure {
        request_data: &data,
        exception: None,
        key: &key,
        traceback: None,
        info: &config,
    };
    async_post_call_failure_hook(&lifecycle, &call).expect("failure hook");
    assert!(
        rig.spans().is_empty(),
        "without a parent the hook touches nothing, guardrails included"
    );
}

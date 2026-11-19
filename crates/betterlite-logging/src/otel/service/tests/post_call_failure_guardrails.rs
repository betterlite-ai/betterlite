use super::fixtures::{error_config, guardrail_entry, request_data};
use super::support::{Rig, START, instant, text_attribute};
use crate::otel::service::post_call::{
    PostCallFailure, ProxyCallKey, async_post_call_failure_hook,
};

#[rstest::rstest]
#[case::hook_first(true)]
#[case::emitter_first(false)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(#[case] hook_first: bool) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let data = request_data("call-g", vec![guardrail_entry()]);
    let key = ProxyCallKey {
        parent_span: Some(&parent),
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
    if hook_first {
        async_post_call_failure_hook(&lifecycle, &call).expect("failure hook");
    }
    lifecycle
        .emit_request_guardrails(&data, Some(&parent))
        .expect("request data guardrails");
    if !hook_first {
        async_post_call_failure_hook(&lifecycle, &call).expect("failure hook");
    }
    let spans = rig.named("guardrail");
    assert_eq!(spans.len(), 1, "one invocation, one span");
    let span = &spans[0];
    assert_eq!(span.parent_span_id, parent.context.span_id());
    assert_eq!(span.start_time, instant(1_704_067_200.5));
    assert_eq!(span.end_time, instant(1_704_067_200.75));
    assert_eq!(text_attribute(span, "guardrail_name"), "pii");
    assert_eq!(
        text_attribute(span, "metadata.user_api_key_team_id"),
        "team-a"
    );
}

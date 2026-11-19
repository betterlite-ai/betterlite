use super::fixtures::{details, guardrail_entry};
use super::support::{Rig, START};
use crate::otel::service::post_call::async_post_call_success_hook;

#[derive(Clone, Copy)]
enum Argument {
    Present,
    Absent,
}

// _get_span_context reads the kwargs carrier, not the parent_otel_span the hook also receives
#[rstest::rstest]
#[case::metadata_parent_wins(Argument::Present)]
#[case::metadata_parent_without_argument(Argument::Absent)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(#[case] mode: Argument) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let carrier = Rig::detached(&lifecycle);
    let call_details = details("call-c", Some(vec![guardrail_entry()]), Some(&carrier));
    let argument = match mode {
        Argument::Present => Some(rig.parent(&lifecycle, "proxy", START)),
        Argument::Absent => None,
    };
    async_post_call_success_hook(&lifecycle, &call_details, argument.as_ref())
        .expect("success hook");
    let guardrails = rig.named("guardrail");
    assert_eq!(guardrails.len(), 1);
    assert_eq!(guardrails[0].parent_span_id, carrier.context.span_id());
    if let Some(span) = argument {
        assert_ne!(guardrails[0].parent_span_id, span.context.span_id());
    }
}

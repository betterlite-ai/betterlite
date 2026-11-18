use opentelemetry::trace::SpanId;

use super::fixtures::{details, guardrail_entry};
use super::support::{Rig, START, instant, text_attribute};
use crate::otel::service::post_call::async_post_call_success_hook;

#[derive(Clone, Copy)]
enum Anchor {
    Live,
    Detached,
    NoGuardrails,
}

#[rstest::rstest]
#[case::live_parent(Anchor::Live)]
#[case::detached_parent(Anchor::Detached)]
#[case::no_guardrails(Anchor::NoGuardrails)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(#[case] anchor: Anchor) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let span = match anchor {
        Anchor::Live | Anchor::NoGuardrails => rig.parent(&lifecycle, "proxy", START),
        Anchor::Detached => rig.detached(&lifecycle),
    };
    let entries = match anchor {
        Anchor::Live | Anchor::Detached => Some(vec![guardrail_entry()]),
        Anchor::NoGuardrails => None,
    };
    let details = details("call-s", entries, Some(&span));
    async_post_call_success_hook(&lifecycle, &details, Some(&span)).expect("success hook");
    let spans = rig.named("guardrail");
    match anchor {
        Anchor::NoGuardrails => assert!(spans.is_empty()),
        Anchor::Live => {
            assert_eq!(spans.len(), 1);
            assert_eq!(spans[0].parent_span_id, span.context.span_id());
            assert_eq!(spans[0].start_time, instant(1_704_067_200.5));
            assert_eq!(spans[0].end_time, instant(1_704_067_200.75));
            assert_eq!(text_attribute(&spans[0], "guardrail_name"), "pii");
            assert_eq!(
                text_attribute(&spans[0], "metadata.user_api_key_team_id"),
                "team-a"
            );
        }
        Anchor::Detached => {
            assert_eq!(spans.len(), 1, "a traceparent parent still gets the span");
            assert_eq!(spans[0].parent_span_id, SpanId::INVALID);
        }
    }
}

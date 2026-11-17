#![cfg(test)]

use opentelemetry::Context;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{TraceContextExt as _, Tracer as _};

use betterlite_logging::otel::model::baggage::TEAM_ID;
use betterlite_logging::otel::model::metadata::wire::callback::RequestData;
use betterlite_logging::otel::model::metadata::wire::payload::{Auth, Metadata};
use betterlite_logging::otel::plumbing::context::context_slots::RequestTraceContext;
use betterlite_logging::otel::v2_callback::identity::{self, IdentitySeed};
use betterlite_logging::otel::v2_callback::proxy;
use betterlite_logging::otel::v2_callback::registered;

use super::support::{harness, settings};

fn team_auth() -> Auth {
    Auth {
        team_id: Some("team-1".to_owned()),
        team_alias: None,
        team_metadata: None,
        key_hash: None,
        end_user_id: None,
        user_id: None,
        org_id: None,
        key_alias: None,
    }
}

#[test]
fn seeds_baggage_and_stamps_the_ambient_span() {
    let harness = harness(settings());
    let span = harness.span("server");
    let auth = team_auth();
    let seed = IdentitySeed {
        auth: &auth,
        model: None,
        request_metadata: None,
        ambient_span: Some(&span),
    };
    let mut trace = RequestTraceContext::default();
    let seeded = identity::seed_request_identity(&harness.callback, &seed, &mut trace);
    assert_eq!(
        seeded.baggage().get(TEAM_ID).map(ToString::to_string),
        Some("team-1".to_owned())
    );
    assert!(span.has_attribute(TEAM_ID));
    assert_eq!(
        trace.root.as_ref().map(|root| root.context.span_id()),
        Some(span.context.span_id())
    );
}

#[test]
fn the_anchored_root_wins_over_the_ambient_span() {
    let harness = harness(settings());
    let root = harness.span("root");
    let ambient = harness.span("ambient");
    let mut trace = RequestTraceContext::default();
    proxy::mark_request_root_span(&harness.callback, &root, None, &mut trace);
    let auth = team_auth();
    let seed = IdentitySeed {
        auth: &auth,
        model: None,
        request_metadata: None,
        ambient_span: Some(&ambient),
    };
    let _seeded = identity::seed_request_identity(&harness.callback, &seed, &mut trace);
    assert!(root.has_attribute(TEAM_ID));
    assert!(!ambient.has_attribute(TEAM_ID));
    assert_eq!(
        trace
            .root
            .as_ref()
            .map(|anchored| anchored.context.span_id()),
        Some(root.context.span_id())
    );
}

#[test]
fn the_attached_guard_carries_identity_into_a_later_span() {
    let harness = harness(settings());
    let auth = team_auth();
    let seed = IdentitySeed {
        auth: &auth,
        model: None,
        request_metadata: None,
        ambient_span: None,
    };
    let mut trace = RequestTraceContext::default();
    let seeded = identity::seed_request_identity(&harness.callback, &seed, &mut trace);
    let _guard = seeded.attach();
    let span = harness.callback.logger.emitter.tracer.start("after-seed");
    let carried = Context::current_with_span(span);
    assert_eq!(
        carried.baggage().get(TEAM_ID).map(ToString::to_string),
        Some("team-1".to_owned())
    );
}

#[test]
fn an_empty_baggage_leaves_the_context_alone() {
    let harness = harness(settings());
    let span = harness.span("server");
    let auth = Auth::default();
    let seed = IdentitySeed {
        auth: &auth,
        model: None,
        request_metadata: None,
        ambient_span: Some(&span),
    };
    let mut trace = RequestTraceContext::default();
    let seeded = identity::seed_request_identity(&harness.callback, &seed, &mut trace);
    assert!(seeded.baggage().get(TEAM_ID).is_none());
    assert!(trace.root.is_none());
    assert!(!span.has_attribute(TEAM_ID));
}

#[test]
fn the_module_entry_seeds_through_the_registered_logger() {
    let harness = harness(settings());
    let auth = team_auth();
    let seed = IdentitySeed {
        auth: &auth,
        model: None,
        request_metadata: None,
        ambient_span: None,
    };
    let mut trace = RequestTraceContext::default();
    let seeded = registered::seed_request_identity(&harness.registry, &seed, &mut trace)
        .expect("the registered logger seeds");
    assert_eq!(
        seeded.baggage().get(TEAM_ID).map(ToString::to_string),
        Some("team-1".to_owned())
    );
}

#[test]
fn async_pre_call_hook_promotes_the_request_model() {
    let mut config = settings();
    config.baggage_promoted_keys = vec!["gen_ai.request.model".to_owned()];
    let harness = harness(config);
    let span = harness.span("server");
    let auth = Auth::default();
    let data = RequestData {
        model: Some("gpt-4o".to_owned()),
        metadata: Metadata::default(),
        litellm_metadata: Metadata::default(),
    };
    let mut trace = RequestTraceContext::default();
    let seeded =
        identity::async_pre_call_hook(&harness.callback, &auth, &data, Some(&span), &mut trace);
    assert_eq!(
        seeded
            .baggage()
            .get("gen_ai.request.model")
            .map(ToString::to_string),
        Some("gpt-4o".to_owned())
    );
    assert!(span.has_attribute("gen_ai.request.model"));
}

use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::logged_relay_shape;
use betterlite_openai::passthrough::transformation::types::RelayedResponse;

use crate::passthrough::logging::{FOUNDRY_RELAY_SHAPES, FoundryRelayedLog};
use crate::passthrough::plan::FoundryLogInput;
use crate::passthrough::tests::{RecordingLogging, build_plan, object, reply, target};

use super::RES;

pub(super) fn foundry_relay_shape_literals() {
    relay_shapes_log_their_call_types();
    relay_shapes_reject_other_shapes();
    relay_fallbacks_log_the_standard_body();
    undecodable_standard_bodies_log_nothing();
    the_trait_path_logs_a_relay_shape();
}

const RERANK_BODY: &[u8] =
    br#"{"id":"r","results":[{"index":0,"relevance_score":0.5,"document":{"text":"x"}}]}"#;
const RERANK_BODY_WITHOUT_SCORE: &[u8] = br#"{"results":[{"index":0}]}"#;
const FLUX_BODY: &[u8] = br#"{"created":1,"data":[{"url":"https://image.test/one.png"}]}"#;
const FLUX_BODY_WITH_WRONG_CREATED: &[u8] = br#"{"created":"yesterday"}"#;

fn relay_shapes_log_their_call_types() {
    let mut rerank_logging = RecordingLogging::default();
    let rerank_parsed = logged_relay_shape(
        &FOUNDRY_RELAY_SHAPES,
        200_u16,
        RERANK_BODY,
        &mut rerank_logging,
        "providers/cohere/v2/rerank",
    );
    assert!(rerank_parsed.is_some());
    assert_eq!(rerank_logging.call_type.as_deref(), Some("arerank"));
    let mut flux_logging = RecordingLogging::default();
    let flux_parsed = logged_relay_shape(
        &FOUNDRY_RELAY_SHAPES,
        200_u16,
        FLUX_BODY,
        &mut flux_logging,
        "/providers/blackforestlabs/v1/flux-2-pro",
    );
    assert!(flux_parsed.is_some());
    assert_eq!(flux_logging.call_type.as_deref(), Some("aimage_generation"));
}

fn relay_shapes_reject_other_shapes() {
    for (status, body, endpoint) in [
        (
            201_u16,
            FLUX_BODY,
            "/providers/blackforestlabs/v1/flux-2-pro",
        ),
        (200_u16, RERANK_BODY_WITHOUT_SCORE, "/rerank"),
        (200_u16, FLUX_BODY, "/providers/blackforestlabs/v1/flux-2"),
        (200_u16, FLUX_BODY, "/rerankx"),
        (
            200_u16,
            FLUX_BODY_WITH_WRONG_CREATED,
            "/providers/blackforestlabs/v1/flux-2-pro",
        ),
    ] {
        let mut logging = RecordingLogging::default();
        let parsed =
            logged_relay_shape(&FOUNDRY_RELAY_SHAPES, status, body, &mut logging, endpoint);
        assert!(parsed.is_none(), "{endpoint} {status}");
        assert_eq!(logging.call_type, None);
    }
}

fn relay_fallbacks_log_the_standard_body() {
    let plan = build_plan(None, None, Some(RES), None, Object::new());
    let mut logging = RecordingLogging::default();
    let logged = plan.logging_relayed_response(
        &FoundryLogInput {
            target: &target("gpt-x", "/providers/blackforestlabs/v1/flux-2-pro"),
            reply: &reply(200_u16, &Headers::new(), FLUX_BODY),
            request_data: &object("{}"),
            request_url: "https://res.test/flux",
        },
        &mut logging,
    );
    assert!(matches!(logged, Some(FoundryRelayedLog::Relay(_))));
    assert_eq!(logging.call_type.as_deref(), Some("aimage_generation"));
    let mut standard_logging = RecordingLogging::default();
    let standard_logged = plan.logging_relayed_response(
        &FoundryLogInput {
            target: &target("gpt-x", "unknown/route"),
            reply: &reply(200_u16, &Headers::new(), b"[]"),
            request_data: &object("{}"),
            request_url: "https://res.test/unknown",
        },
        &mut standard_logging,
    );
    let Some(FoundryRelayedLog::Standard(response)) = standard_logged else {
        panic!("the relayed body is the standard fallback");
    };
    assert_eq!(response.response, DecisionsJson::Text(String::from("[]")));
}

fn undecodable_standard_bodies_log_nothing() {
    let plan = build_plan(None, None, Some(RES), None, Object::new());
    let logged = plan.logging_relayed_response(
        &FoundryLogInput {
            target: &target("gpt-x", "unknown/route"),
            reply: &reply(200_u16, &Headers::new(), b"\xff\xfe"),
            request_data: &object("{}"),
            request_url: "https://res.test/unknown",
        },
        &mut RecordingLogging::default(),
    );
    assert!(logged.is_none());
}

// the trait path delegates its relay call type through TraitFoundryLogging
fn the_trait_path_logs_a_relay_shape() {
    let plan = build_plan(None, None, Some(RES), None, Object::new());
    let mut logging = RecordingLogging::default();
    let response = RelayedResponse {
        status_code: 200_u16,
        body: RERANK_BODY,
    };
    let logged = plan.logging_non_streaming_response(
        &target("gpt-x", "providers/cohere/v2/rerank"),
        &response,
        &object("{}"),
        &mut logging,
    );
    assert!(matches!(logged, Some(FoundryRelayedLog::Relay(_))));
    assert_eq!(logging.call_type.as_deref(), Some("arerank"));
}

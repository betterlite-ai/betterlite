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
    relay_shapes_accept_their_declared_fields();
    relay_shapes_reject_other_shapes();
    relay_fallbacks_log_the_standard_body();
    undecodable_standard_bodies_log_nothing();
    the_trait_path_logs_a_relay_shape();
}

const RERANK_BODY: &[u8] =
    br#"{"id":"r","results":[{"index":0,"relevance_score":0.5,"document":{"text":"x"}}]}"#;
const FLUX_BODY: &[u8] = br#"{"created":1,"data":[{"url":"https://image.test/one.png"}]}"#;
const RERANK_ENDPOINT: &str = "/rerank";
const FLUX_ENDPOINT: &str = "/providers/blackforestlabs/v1/flux-2-pro";

fn relayed_shape(body: &[u8], endpoint: &str) -> (Option<Object>, Option<String>) {
    let mut logging = RecordingLogging::default();
    let parsed = logged_relay_shape(&FOUNDRY_RELAY_SHAPES, 200_u16, body, &mut logging, endpoint);
    (parsed, logging.call_type)
}

// the parsed shape is the body itself, so spend runs on every field the provider sent
fn assert_shape_logged(body: &[u8], endpoint: &str, call_type: &str) {
    let (parsed, logged) = relayed_shape(body, endpoint);
    let expected: Object = serde_json::from_slice(body).unwrap();
    assert_eq!(parsed, Some(expected), "{endpoint} {body:?}");
    assert_eq!(logged.as_deref(), Some(call_type), "{endpoint} {body:?}");
}

fn assert_shape_rejected(body: &[u8], endpoint: &str) {
    let (parsed, logged) = relayed_shape(body, endpoint);
    assert_eq!(parsed, None, "{endpoint} {body:?}");
    assert_eq!(logged, None, "{endpoint} {body:?}");
}

fn relay_shapes_log_their_call_types() {
    assert_shape_logged(RERANK_BODY, "providers/cohere/v2/rerank", "arerank");
    assert_shape_logged(FLUX_BODY, FLUX_ENDPOINT, "aimage_generation");
}

fn relay_shapes_accept_their_declared_fields() {
    for body in [
        &b"{}"[..],
        &br#"{"id":null,"results":null,"meta":null}"#[..],
        &br#"{"id":"x","meta":{}}"#[..],
        &br#"{"results":[{"index":-1,"relevance_score":0}]}"#[..],
        &br#"{"results":[{"index":9223372036854775808,"relevance_score":0.5}]}"#[..],
        &br#"{"results":[{"index":0,"relevance_score":1,"document":{"text":"x","extra":1}}]}"#[..],
        &br#"{"unknown":true,"results":[{"index":0,"relevance_score":0.5}]}"#[..],
    ] {
        assert_shape_logged(body, RERANK_ENDPOINT, "arerank");
    }
    for body in [
        &br#"{"created":1}"#[..],
        &br#"{"created":1,"data":null}"#[..],
        &br#"{"created":1,"data":[]}"#[..],
        &br#"{"created":-1,"data":[]}"#[..],
        &br#"{"created":9223372036854775808,"data":[]}"#[..],
        &br#"{"created":1,"data":[{}]}"#[..],
        &br#"{"created":1,"data":[{"url":null,"b64_json":null,"revised_prompt":null,"extra":1}]}"#
            [..],
        &br#"{"created":1,"unknown":{"a":1}}"#[..],
    ] {
        assert_shape_logged(body, FLUX_ENDPOINT, "aimage_generation");
    }
}

fn relay_shapes_reject_other_shapes() {
    for body in [
        &br#"{"results":[{"index":0}]}"#[..],
        &br#"{"results":[{"relevance_score":0.5}]}"#[..],
        &br#"{"results":[{"index":0.5,"relevance_score":0.5}]}"#[..],
        &br#"{"results":[{"index":"1","relevance_score":0.5}]}"#[..],
        &br#"{"results":[{"index":0,"relevance_score":"x"}]}"#[..],
        &br#"{"results":[{"index":0,"relevance_score":true}]}"#[..],
        &br#"{"results":[{"index":0,"relevance_score":0.5,"document":null}]}"#[..],
        &br#"{"results":[{"index":0,"relevance_score":0.5,"document":{}}]}"#[..],
        &br#"{"results":[{"index":0,"relevance_score":0.5,"document":{"text":5}}]}"#[..],
        &br#"{"results":{}}"#[..],
        &br#"{"results":[5]}"#[..],
        &br#"{"id":5}"#[..],
        &br#"{"meta":[]}"#[..],
    ] {
        assert_shape_rejected(body, RERANK_ENDPOINT);
    }
    for body in [
        &br#"{"created":"yesterday"}"#[..],
        &br#"{"data":[]}"#[..],
        &br#"{"created":null}"#[..],
        &br#"{"created":1.5}"#[..],
        &br#"{"created":true}"#[..],
        &br#"{"created":1,"data":{}}"#[..],
        &br#"{"created":1,"data":[5]}"#[..],
        &br#"{"created":1,"data":[{"url":5}]}"#[..],
        &br#"{"created":1,"data":[{"b64_json":5}]}"#[..],
        &br#"{"created":1,"data":[{"revised_prompt":5}]}"#[..],
    ] {
        assert_shape_rejected(body, FLUX_ENDPOINT);
    }
    // the suffix must end the relayed path, and only a 200 carries a relayed object
    assert_shape_rejected(FLUX_BODY, "/providers/blackforestlabs/v1/flux-2");
    assert_shape_rejected(FLUX_BODY, "/rerankx");
    let mut logging = RecordingLogging::default();
    let created = logged_relay_shape(
        &FOUNDRY_RELAY_SHAPES,
        201_u16,
        FLUX_BODY,
        &mut logging,
        FLUX_ENDPOINT,
    );
    assert_eq!(created, None);
    assert_eq!(logging.call_type, None);
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

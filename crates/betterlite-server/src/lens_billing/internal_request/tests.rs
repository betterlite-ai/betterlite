#![cfg(test)]

use std::net::SocketAddr;
use std::sync::Arc;

use betterlite_auth::reservation_settlement::handle::ReservationHandle;
use betterlite_logging::logging_privacy::PrivacyContext;
use rust_decimal::Decimal;

use super::{ClientIp, Incoming, MessagePrivacy, MessagePrivacyScope, build_request};
use crate::lens_inference::request_body::{AnalysisBody, RequestIds};

struct ForwardedIp;

impl ClientIp for ForwardedIp {
    fn get_mcp_client_ip(&self, incoming: &Incoming) -> Option<String> {
        incoming.x_forwarded_for.clone()
    }
}

fn sample_body() -> AnalysisBody<'static> {
    AnalysisBody::new(
        "lens-model",
        "hello",
        RequestIds {
            lens_id: "lens-1",
            run_id: "job-1",
            worker_id: "worker-1",
            team_id: None,
        },
    )
}

fn incoming() -> Incoming {
    Incoming {
        scheme: Some("https".to_owned()),
        client: Some(SocketAddr::from(([10, 0, 0, 1], 4321))),
        x_forwarded_for: Some("203.0.113.7".to_owned()),
        x_forwarded_proto: Some("https".to_owned()),
        host: Some("worker.example".to_owned()),
        privacy: MessagePrivacy::default(),
    }
}

#[test]
fn the_request_is_a_post_to_the_chat_completions_route() {
    let request = build_request(&sample_body(), &incoming(), &ForwardedIp).unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(request.query, "");
    assert_eq!(
        request.headers,
        vec![("content-type".to_owned(), "application/json".to_owned())]
    );
    assert_eq!(request.server, ("litellm.internal".to_owned(), 80));
}

#[test]
fn the_scheme_defaults_to_http_and_keeps_a_given_one() {
    let mut plain = incoming();
    plain.scheme = None;
    let request = build_request(&sample_body(), &plain, &ForwardedIp).unwrap();
    assert_eq!(request.scheme, "http");
    plain.scheme = Some(String::new());
    let request = build_request(&sample_body(), &plain, &ForwardedIp).unwrap();
    assert_eq!(request.scheme, "http");
    let request = build_request(&sample_body(), &incoming(), &ForwardedIp).unwrap();
    assert_eq!(request.scheme, "https");
}

#[test]
fn the_client_carries_the_resolved_address_and_the_peer_port() {
    let request = build_request(&sample_body(), &incoming(), &ForwardedIp).unwrap();
    assert_eq!(request.client, Some(("203.0.113.7".to_owned(), 4321_u16)));
    let mut unknown_peer = incoming();
    unknown_peer.client = None;
    let request = build_request(&sample_body(), &unknown_peer, &ForwardedIp).unwrap();
    assert_eq!(request.client, Some(("203.0.113.7".to_owned(), 0_u16)));
}

#[test]
fn a_missing_or_empty_client_address_sends_no_client() {
    let mut missing = incoming();
    missing.x_forwarded_for = None;
    let request = build_request(&sample_body(), &missing, &ForwardedIp).unwrap();
    assert_eq!(request.client, None);
    let mut empty = incoming();
    empty.x_forwarded_for = Some(String::new());
    let request = build_request(&sample_body(), &empty, &ForwardedIp).unwrap();
    assert_eq!(request.client, None);
}

#[test]
fn the_reservation_cell_keeps_the_last_stamp() {
    let request = build_request(&sample_body(), &incoming(), &ForwardedIp).unwrap();
    assert!(request.reservation.get().is_none());
    let first = ReservationHandle::new(Decimal::ONE, Vec::new(), Decimal::ZERO);
    let second = ReservationHandle::new(Decimal::TWO, Vec::new(), Decimal::ZERO);
    request.reservation.set(first.clone());
    request.reservation.set(second.clone());
    let kept = request.reservation.get().unwrap();
    assert!(Arc::ptr_eq(&kept.0, &second.0));
    assert!(!Arc::ptr_eq(&kept.0, &first.0));
}

#[test]
fn the_privacy_scope_restores_the_previous_context() {
    let privacy = Incoming::default().privacy;
    assert!(!privacy.context().message_logging_disabled);
    let outer = MessagePrivacyScope::enter(
        &privacy,
        PrivacyContext {
            message_logging_disabled: true,
        },
    );
    assert!(privacy.context().message_logging_disabled);
    let inner = MessagePrivacyScope::enter(
        &privacy,
        PrivacyContext {
            message_logging_disabled: false,
        },
    );
    assert!(!privacy.context().message_logging_disabled);
    drop(inner);
    assert!(privacy.context().message_logging_disabled);
    drop(outer);
    assert!(!privacy.context().message_logging_disabled);
}

#[test]
fn the_request_carries_the_worker_request_privacy_cell() {
    let worker = incoming();
    let request = build_request(&sample_body(), &worker, &ForwardedIp).unwrap();
    let scope = MessagePrivacyScope::enter(
        &worker.privacy,
        PrivacyContext {
            message_logging_disabled: true,
        },
    );
    assert!(request.privacy.context().message_logging_disabled);
    drop(scope);
    assert!(!request.privacy.context().message_logging_disabled);
}

#![cfg(test)]

use std::sync::Arc;

use betterlite_anthropic::api::messages_errors::decode::WireDecodeLimits;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use bytes::Bytes;

use super::super::failure::failure_exception;
use super::super::sse::Emitted;
use super::super::tail::ANTHROPIC_PING_CHUNK;
use super::{
    HookConfig, Rig, StreamFailure, abort_of, call, drive, other_failure, sse_body, status_failure,
    text_chunk,
};
use crate::sse_data_generator::types::{FailureKind, FailureParam, FailureType};
use crate::stream_chunks::StreamItem;

fn error_frame(message: &str, kind: &str) -> String {
    format!(
        "event: error\ndata: {{\"type\": \"error\", \"error\": {{\"type\": \"{kind}\", \"message\": \"{message}\"}}}}\n\n"
    )
}

fn closed_frame() -> Result<StreamItem, StreamFailure> {
    Ok(StreamItem::Bytes(Bytes::from_static(b"data: x\n\n")))
}

#[test]
fn error_frame_after_closed_frame() {
    let rig = Rig::new(HookConfig::plain());
    let items = vec![closed_frame(), Err(status_failure("slow down", 429))];
    let results = drive(sse_body(&rig.generator, call(items), None));
    assert_eq!(results.len(), 2);
    match results.get(1) {
        Some(Ok(Emitted::AnthropicError(frame))) => {
            assert_eq!(frame.status_code, 429);
            assert_eq!(frame.body, error_frame("slow down", "rate_limit_error"));
        }
        Some(Ok(Emitted::Frame(_))) => panic!("expected the typed error frame"),
        Some(Err(_)) => panic!("expected the typed error frame"),
        None => panic!("no frame"),
    }
    assert_eq!(
        rig.events(),
        vec![
            String::from("wrap:requested"),
            String::from("failure:slow down"),
        ]
    );
    let outcomes = rig.outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].stream_completed);
}

#[test]
fn error_seals_after_torn_bytes() {
    let rig = Rig::new(HookConfig::plain());
    let items = vec![
        Ok(text_chunk("data: {")),
        Err(status_failure("slow down", 429)),
    ];
    let results = drive(sse_body(&rig.generator, call(items), None));
    assert_eq!(results.len(), 2);
    let expected = format!(
        "\n{ANTHROPIC_PING_CHUNK}{}",
        error_frame("slow down", "rate_limit_error")
    );
    match results.get(1) {
        Some(Ok(Emitted::Frame(bytes))) => assert_eq!(bytes, &Bytes::from(expected)),
        Some(Ok(Emitted::AnthropicError(_))) => panic!("a sealed frame is plain bytes"),
        Some(Err(_)) => panic!("expected a frame"),
        None => panic!("no frame"),
    }
}

#[test]
fn no_deployments_becomes_429() {
    let rig = Rig::new(HookConfig::plain());
    let items = vec![
        closed_frame(),
        Err(status_failure(
            "No deployments available for selected model",
            500,
        )),
    ];
    let results = drive(sse_body(&rig.generator, call(items), None));
    match results.get(1) {
        Some(Ok(Emitted::AnthropicError(frame))) => {
            assert_eq!(frame.status_code, 429);
            assert_eq!(
                frame.body,
                error_frame(
                    "No deployments available for selected model",
                    "rate_limit_error"
                )
            );
        }
        Some(Ok(Emitted::Frame(_))) => panic!("expected the typed error frame"),
        Some(Err(_)) => panic!("expected the typed error frame"),
        None => panic!("no frame"),
    }
}

#[test]
fn tag_routing_becomes_401() {
    let rig = Rig::new(HookConfig::plain());
    let items = vec![
        closed_frame(),
        Err(status_failure(
            "Not allowed to access model due to tags configuration",
            400,
        )),
    ];
    let results = drive(sse_body(&rig.generator, call(items), None));
    match results.get(1) {
        Some(Ok(Emitted::AnthropicError(frame))) => {
            assert_eq!(frame.status_code, 401);
            assert_eq!(
                frame.body,
                error_frame(
                    "Not allowed to access model due to tags configuration",
                    "authentication_error"
                )
            );
        }
        Some(Ok(Emitted::Frame(_))) => panic!("expected the typed error frame"),
        Some(Err(_)) => panic!("expected the typed error frame"),
        None => panic!("no frame"),
    }
}

#[test]
fn failure_exception_shapes() {
    let mut failure = status_failure("slow down", 429);
    failure.error_type = FailureType::Text(String::from("RateLimitError"));
    failure.param = FailureParam::Text(String::from("messages"));
    let exception = failure_exception(&failure, None);
    assert_eq!(exception.message, "slow down");
    assert_eq!(exception.error_type, "RateLimitError");
    assert_eq!(exception.param, Some(String::from("messages")));
    assert_eq!(exception.code, "429");
    assert_eq!(exception.openai_code, Some(String::from("429")));
}

#[test]
fn missing_type_uses_status() {
    let exception = failure_exception(&status_failure("nope", 500), None);
    assert_eq!(exception.error_type, "internal_server_error");
    assert_eq!(exception.param, None);
    assert_eq!(exception.code, "500");
    let unauthorized = failure_exception(&status_failure("nope", 401), None);
    assert_eq!(unauthorized.error_type, "authentication_error");
}

#[test]
fn failure_exception_defaults_status() {
    let exception = failure_exception(&other_failure("nope"), None);
    assert_eq!(exception.code, "500");
    assert_eq!(exception.error_type, "internal_server_error");
    assert_eq!(exception.openai_code, Some(String::from("500")));
}

#[test]
fn failure_redacts_message() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor");
    let secret = "Bearer abcdefghij1234567890";
    let redacted = failure_exception(&other_failure(secret), Some(&redactor));
    assert_eq!(redacted.message, "REDACTED");
    let plain = failure_exception(&other_failure(secret), None);
    assert_eq!(plain.message, secret);
}

#[test]
fn generator_redacts_failure() {
    let mut rig = Rig::new(HookConfig::plain());
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor");
    rig.generator.redactor = Some(Arc::new(redactor));
    let failure = status_failure("Bearer abcdefghij1234567890", 400);
    let results = drive(sse_body(&rig.generator, call(vec![Err(failure)]), None));
    match results.first() {
        Some(Ok(Emitted::AnthropicError(frame))) => {
            assert_eq!(frame.body, error_frame("REDACTED", "invalid_request_error"));
        }
        Some(Ok(Emitted::Frame(_))) => panic!("expected the typed error frame"),
        Some(Err(_)) => panic!("expected the typed error frame"),
        None => panic!("no frame"),
    }
}

#[test]
fn http_failure_aborts_body() {
    let rig = Rig::new(HookConfig::plain());
    let mut failure = status_failure("blocked", 400);
    failure.kind = FailureKind::Http;
    let items = vec![Ok(text_chunk("x")), Err(failure)];
    let results = drive(sse_body(&rig.generator, call(items), None));
    assert_eq!(results.len(), 2);
    let abort = abort_of(&results, 1);
    assert_eq!(abort.status, 400);
    assert_eq!(abort.detail, "blocked");
    let outcomes = rig.outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(!outcomes[0].stream_completed);
    assert!(!outcomes[0].client_disconnected);
    assert_eq!(rig.releases().len(), 0);
}

#[test]
fn http_failure_status_defaults() {
    let rig = Rig::new(HookConfig::plain());
    let mut failure = other_failure("gone");
    failure.kind = FailureKind::Http;
    let results = drive(sse_body(&rig.generator, call(vec![Err(failure)]), None));
    let abort = abort_of(&results, 0);
    assert_eq!(abort.status, 500);
    assert_eq!(abort.detail, "gone");
}

#[test]
fn wire_limit_failure_aborts_the_body() {
    let mut rig = Rig::new(HookConfig::plain());
    rig.generator.limits = WireDecodeLimits {
        max_json_bytes: 4,
        max_event_bytes: 4,
        max_items: 100,
    };
    let results = drive(sse_body(
        &rig.generator,
        call(vec![Err(status_failure("a long message", 400))]),
        None,
    ));
    let abort = abort_of(&results, 0);
    assert_eq!(abort.status, 500);
    assert_eq!(abort.detail, "messages JSON exceeds the configured limit");
    let outcomes = rig.outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].stream_completed);
    assert!(!outcomes[0].client_disconnected);
}

#[test]
fn hook_replacement_aborts() {
    let mut replacement = status_failure("blocked by hook", 503);
    replacement.kind = FailureKind::Http;
    let mut config = HookConfig::plain();
    config.replacement = Some(replacement);
    let rig = Rig::new(config);
    let results = drive(sse_body(
        &rig.generator,
        call(vec![Err(other_failure("provider exploded"))]),
        None,
    ));
    let abort = abort_of(&results, 0);
    assert_eq!(abort.status, 503);
    assert_eq!(abort.detail, "blocked by hook");
    assert_eq!(
        rig.events(),
        vec![
            String::from("wrap:requested"),
            String::from("failure:provider exploded"),
        ]
    );
}

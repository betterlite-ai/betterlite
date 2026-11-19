use std::sync::{Arc, PoisonError};

use betterlite_azure::passthrough::logging::AzureRelayedLog;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::RelayedResponse;

use crate::passthrough::logging::FoundryRelayedLog;
use crate::passthrough::plan::FoundryLogInput;
use crate::passthrough::tests::{
    OCR_ENDPOINT, OCR_MODEL, OcrScript, RecordingChat, RecordingLogging, RecordingOcr, build_plan,
    capture_warnings, object, plan_with_reply, reply, target,
};

use super::{OCR_BODY, RES};

pub(super) fn ocr_usage_log() {
    azure_chat_projection_runs_first();
    azure_relay_projection_runs_before_ocr();
    ocr_route_logs_the_decoded_reply();
    native_ocr_binding_decodes();
    ocr_decodes_only_a_200();
    ocr_failure_warns_and_falls_back();
    an_absent_ocr_body_logs_nothing();
    a_malformed_ocr_body_warns();
    the_trait_path_logs_ocr_without_the_ocr_call_type();
}

pub(super) fn scripted_ocr() -> Arc<RecordingOcr> {
    let response = serde_json::from_str(OCR_BODY).expect("OCR fixture");
    let script = OcrScript::Response(response);
    RecordingOcr::scripted(script)
}

fn azure_chat_projection_runs_first() {
    let headers = Headers::new();
    let empty = object("{}");
    let chat = RecordingChat::default();
    let chat_calls = chat.calls.clone();
    let mut projecting = plan_with_reply(chat);
    projecting.ocr = Some(scripted_ocr());
    let mut logging = RecordingLogging::default();
    let logged = projecting.logging_relayed_response(
        &FoundryLogInput {
            target: &target("gpt-4o", "chat/completions"),
            reply: &reply(200_u16, &headers, OCR_BODY.as_bytes()),
            request_data: &empty,
            request_url: "https://res.test/chat/completions",
        },
        &mut logging,
    );
    assert!(matches!(
        logged,
        Some(FoundryRelayedLog::Azure(AzureRelayedLog::Chat(_)))
    ));
    assert_eq!(
        chat_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        1_usize
    );
    assert_eq!(logging.call_type, None);
}

fn azure_relay_projection_runs_before_ocr() {
    let ocr = scripted_ocr();
    let mut plan = build_plan(None, None, None, None, Object::new());
    plan.ocr = Some(ocr.clone());
    let mut logging = RecordingLogging::default();
    let logged = plan.logging_relayed_response(
        &FoundryLogInput {
            target: &target("gpt-x", "deployments/d/embeddings"),
            reply: &reply(
                200_u16,
                &Headers::new(),
                br#"{"object": "list", "data": []}"#,
            ),
            request_data: &object("{}"),
            request_url: "https://res.test/embeddings",
        },
        &mut logging,
    );
    assert!(matches!(
        logged,
        Some(FoundryRelayedLog::Azure(AzureRelayedLog::Relay(_)))
    ));
    assert_eq!(logging.call_type.as_deref(), Some("aembedding"));
    assert!(
        ocr.calls().is_empty(),
        "the Azure projection runs before OCR"
    );
}

fn ocr_route_logs_the_decoded_reply() {
    let litellm_params = object(r#"{"litellm_metadata": {"model_group": "group-one"}}"#);
    let ocr = scripted_ocr();
    let mut plan = build_plan(None, None, Some(RES), None, litellm_params);
    plan.ocr = Some(ocr.clone());
    let mut logging = RecordingLogging::default();
    let endpoint = format!("group-one/{OCR_ENDPOINT}");
    let request_url = format!("{RES}/{OCR_ENDPOINT}");
    let logged = plan.logging_relayed_response(
        &FoundryLogInput {
            target: &target(OCR_MODEL, &endpoint),
            reply: &reply(200_u16, &Headers::new(), OCR_BODY.as_bytes()),
            request_data: &object("{}"),
            request_url: &request_url,
        },
        &mut logging,
    );
    assert!(matches!(logged, Some(FoundryRelayedLog::Ocr(_))));
    assert_eq!(logging.call_type.as_deref(), Some("aocr"));
    let calls = ocr.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, OCR_MODEL);
    assert_eq!(calls[0].endpoint, OCR_ENDPOINT);
    assert_eq!(calls[0].body.as_slice(), OCR_BODY.as_bytes());
}

fn native_ocr_binding_decodes() {
    let plan = build_plan(None, None, Some(RES), None, Object::new());
    let mut logging = RecordingLogging::default();
    let request_url = format!("{RES}/{OCR_ENDPOINT}");
    let logged = plan.logging_relayed_response(
        &FoundryLogInput {
            target: &target(OCR_MODEL, OCR_ENDPOINT),
            reply: &reply(200_u16, &Headers::new(), OCR_BODY.as_bytes()),
            request_data: &object("{}"),
            request_url: &request_url,
        },
        &mut logging,
    );
    assert!(
        matches!(logged, Some(FoundryRelayedLog::Ocr(_))),
        "the native binding decodes an OCR route"
    );
    assert_eq!(logging.call_type.as_deref(), Some("aocr"));
}

fn ocr_decodes_only_a_200() {
    let ocr = scripted_ocr();
    let mut plan = build_plan(None, None, Some(RES), None, Object::new());
    plan.ocr = Some(ocr.clone());
    let mut logging = RecordingLogging::default();
    let request_url = format!("{RES}/{OCR_ENDPOINT}");
    let logged = plan.logging_relayed_response(
        &FoundryLogInput {
            target: &target(OCR_MODEL, OCR_ENDPOINT),
            reply: &reply(201_u16, &Headers::new(), OCR_BODY.as_bytes()),
            request_data: &object("{}"),
            request_url: &request_url,
        },
        &mut logging,
    );
    assert!(matches!(logged, Some(FoundryRelayedLog::Standard(_))));
    assert_eq!(logging.call_type, None);
    assert!(ocr.calls().is_empty(), "OCR runs only for a 200");
}

fn ocr_failure_warns_and_falls_back() {
    let ocr = RecordingOcr::scripted(OcrScript::Failure);
    let mut plan = build_plan(None, None, Some(RES), None, Object::new());
    plan.ocr = Some(ocr.clone());
    let mut logging = RecordingLogging::default();
    let request_url = format!("{RES}/{OCR_ENDPOINT}");
    let (logged, warnings) = capture_warnings(|| {
        plan.logging_relayed_response(
            &FoundryLogInput {
                target: &target(OCR_MODEL, OCR_ENDPOINT),
                reply: &reply(200_u16, &Headers::new(), OCR_BODY.as_bytes()),
                request_data: &object("{}"),
                request_url: &request_url,
            },
            &mut logging,
        )
    });
    assert!(matches!(logged, Some(FoundryRelayedLog::Standard(_))));
    assert_eq!(logging.call_type, None);
    assert_eq!(ocr.calls().len(), 1);
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0].starts_with(&format!(
            "azure_ai passthrough: OCR body from {RES}/{OCR_ENDPOINT} is not costable:"
        )),
        "{warnings:?}"
    );
    assert!(warnings[0].contains("malformed OCR body"), "{warnings:?}");
}

fn an_absent_ocr_body_logs_nothing() {
    let ocr = RecordingOcr::scripted(OcrScript::Absent);
    let mut plan = build_plan(None, None, Some(RES), None, Object::new());
    plan.ocr = Some(ocr.clone());
    let mut logging = RecordingLogging::default();
    let request_url = format!("{RES}/{OCR_ENDPOINT}");
    let (logged, warnings) = capture_warnings(|| {
        plan.logging_relayed_response(
            &FoundryLogInput {
                target: &target(OCR_MODEL, OCR_ENDPOINT),
                reply: &reply(200_u16, &Headers::new(), OCR_BODY.as_bytes()),
                request_data: &object("{}"),
                request_url: &request_url,
            },
            &mut logging,
        )
    });
    assert!(matches!(logged, Some(FoundryRelayedLog::Standard(_))));
    assert_eq!(logging.call_type, None);
    assert_eq!(ocr.calls().len(), 1);
    assert!(warnings.is_empty());
}

// the trait's logging object has no OCR call type, so the composing route must call the inherent method
fn the_trait_path_logs_ocr_without_the_ocr_call_type() {
    let ocr = scripted_ocr();
    let mut plan = build_plan(None, None, Some(RES), None, Object::new());
    plan.ocr = Some(ocr.clone());
    let mut logging = RecordingLogging::default();
    let response = RelayedResponse {
        status_code: 200_u16,
        body: OCR_BODY.as_bytes(),
    };
    let logged = plan.logging_non_streaming_response(
        &target(OCR_MODEL, OCR_ENDPOINT),
        &response,
        &object("{}"),
        &mut logging,
    );
    assert!(matches!(logged, Some(FoundryRelayedLog::Ocr(_))));
    assert_eq!(logging.call_type, None, "the trait path cannot set aocr");
}

fn a_malformed_ocr_body_warns() {
    let plan = build_plan(None, None, Some(RES), None, Object::new());
    let mut logging = RecordingLogging::default();
    let request_url = format!("{RES}/{OCR_ENDPOINT}");
    let (logged, warnings) = capture_warnings(|| {
        plan.logging_relayed_response(
            &FoundryLogInput {
                target: &target(OCR_MODEL, OCR_ENDPOINT),
                reply: &reply(200_u16, &Headers::new(), br#"{"pages": "nope"}"#),
                request_data: &object("{}"),
                request_url: &request_url,
            },
            &mut logging,
        )
    });
    assert!(matches!(logged, Some(FoundryRelayedLog::Standard(_))));
    assert_eq!(
        warnings.len(),
        1,
        "a native decode failure warns once: {warnings:?}"
    );
    assert!(
        warnings[0].starts_with(&format!(
            "azure_ai passthrough: OCR body from {RES}/{OCR_ENDPOINT} is not costable:"
        )),
        "{warnings:?}"
    );
}

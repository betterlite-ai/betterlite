use std::collections::BTreeMap;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use rstest::rstest;

use crate::api::realtime_http_transport::calls::{CallsCall, calls_post, session_data};
use crate::api::realtime_http_transport::error::RealtimeHttpError;

use super::{
    BOUNDARY, Reply, Request, exchange, fallback_deps, input_untouched, json_object, logging_env,
    logging_state, ok_reply, runtime, test_source,
};

// Three bytes of every cap: the SDP text is longer than the JSON body cap the decoder enforces
const TINY: ApiLimits = ApiLimits {
    max_json_bytes: 3,
    max_event_bytes: 3,
    max_items: 3,
};

fn alloy_session() -> IndexMap<String, Json> {
    json_object(r#"{"voice":"alloy"}"#)
}

fn assert_sdp_multipart_wire(answer: &UpstreamResponse, request: &Request) {
    assert_eq!(answer.status, 200);
    let head = &request.head;
    assert!(head.starts_with("POST /v1/realtime/calls HTTP/1.1"));
    assert!(
        head.to_ascii_lowercase()
            .contains("authorization: bearer ep-test")
    );
    assert!(head.contains(&format!("boundary={BOUNDARY}")));
    assert_eq!(request.body, expected_multipart());
}

fn assert_calls_recorded(state: &LoggingState, base: &str) {
    assert_eq!(
        state.model_call_details.get("input"),
        Some(&Json::Text(String::from("realtime_sdp_offer")))
    );
    let Some(Json::Object(args)) = state.model_call_details.get("additional_args") else {
        panic!("the pre_call keeps its arguments");
    };
    assert_eq!(
        args.get("api_base"),
        Some(&Json::Text(format!("{base}/v1/realtime/calls")))
    );
    assert!(
        matches!(args.get("headers"), Some(Json::Object(headers)) if headers.contains_key("Authorization"))
    );
    assert_eq!(
        args.get("session"),
        Some(&Json::Object(json_object(
            r#"{"voice":"alloy","type":"realtime","model":"m"}"#
        )))
    );
    assert!(!args.contains_key("complete_input_dict"));
}

#[test]
fn calls_send_the_sdp_as_unnamed_multipart_parts() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let session = alloy_session();
    let replies = [Reply {
        status: 200,
        headers: "content-type: application/sdp",
        body: b"v=0\r\n",
        hold: false,
    }];
    let (_, requests, answers) = exchange(&replies, |api_base| {
        runtime().block_on(async {
            let deps = fallback_deps(&source, None, ApiLimits::default());
            let call = CallsCall {
                api_base,
                ephemeral_key: "ep-test",
                sdp: b"v=0\r\n",
                model: Some("m"),
                session: Some(&session),
                extra_headers: None,
                api_version: None,
                timeout: None,
            };
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            calls_post(&deps, &call, &mut log).await
        })
    });
    assert_sdp_multipart_wire(&answers.unwrap(), &requests[0]);
}

#[test]
fn calls_record_the_offer_the_url_and_the_session() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let session = alloy_session();
    let (api_base, _, answers) = exchange(&[ok_reply()], |api_base| {
        runtime().block_on(async {
            let deps = fallback_deps(&source, None, ApiLimits::default());
            let call = CallsCall {
                api_base,
                ephemeral_key: "ep-test",
                sdp: b"v=0\r\n",
                model: Some("m"),
                session: Some(&session),
                extra_headers: None,
                api_version: None,
                timeout: None,
            };
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            calls_post(&deps, &call, &mut log).await
        })
    });
    assert_eq!(answers.unwrap().status, 200);
    assert_calls_recorded(&state, &api_base);
}

fn expected_multipart() -> Vec<u8> {
    let session = b"{\"voice\":\"alloy\",\"type\":\"realtime\",\"model\":\"m\"}";
    let mut body = Vec::new();
    for (name, content_type, data) in [
        ("sdp", "text/plain", b"v=0\r\n".to_vec()),
        ("session", "application/json", session.to_vec()),
    ] {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n").as_bytes(),
        );
        body.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
        body.extend_from_slice(&data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

#[rstest]
#[case(None, Some("m"), r#"{"type":"realtime","model":"m"}"#)]
#[case(Some("{}"), Some("m"), r#"{"type":"realtime","model":"m"}"#)]
#[case(
    Some(r#"{"type":"transcription"}"#),
    Some("m"),
    r#"{"type":"transcription","model":"m"}"#
)]
#[case(
    Some(r#"{"model":null}"#),
    Some("m"),
    r#"{"type":"realtime","model":null}"#
)]
#[case(
    Some(r#"{"type":"transcription","model":"x"}"#),
    None,
    r#"{"type":"transcription","model":"x"}"#
)]
#[case(Some("{}"), Some(""), r#"{"type":"realtime","model":""}"#)]
fn call_session_defaults_fill_only_absent_keys(
    #[case] session: Option<&str>,
    #[case] model: Option<&str>,
    #[case] expected: &str,
) {
    let supplied = session.map(json_object);
    assert_eq!(
        session_data(supplied.as_ref(), model),
        json_object(expected)
    );
}

#[test]
fn calls_extra_headers_override_the_ephemeral_authorization() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let extra = BTreeMap::from([(
        String::from("Authorization"),
        String::from("Bearer caller-token"),
    )]);
    let replies = [ok_reply()];
    let (_, requests, answers) = exchange(&replies, |api_base| {
        runtime().block_on(async {
            let deps = fallback_deps(&source, None, ApiLimits::default());
            let call = CallsCall {
                api_base,
                ephemeral_key: "ep-test",
                sdp: b"v=0\r\n",
                model: None,
                session: None,
                extra_headers: Some(&extra),
                api_version: None,
                timeout: None,
            };
            let mut log = LogContext {
                state: &mut state,
                env: &envelope,
            };
            calls_post(&deps, &call, &mut log).await
        })
    });
    assert_eq!(answers.unwrap().status, 200);
    let lower = requests[0].head.to_ascii_lowercase();
    assert!(lower.contains("authorization: bearer caller-token"));
    assert!(!lower.contains("authorization: bearer ep-test"));
}

#[rstest]
#[case::invalid_utf8(b"\xff\xfe")]
#[case::over_the_cap(b"v=0\r\n")]
fn invalid_or_oversized_sdp_fails_before_the_pre_call(#[case] sdp: &[u8]) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let source = test_source();
    let answer = runtime().block_on(async {
        let deps = fallback_deps(&source, None, TINY);
        let call = CallsCall {
            api_base: "http://127.0.0.1:9",
            ephemeral_key: "ep-test",
            sdp,
            model: None,
            session: None,
            extra_headers: None,
            api_version: None,
            timeout: None,
        };
        let mut log = LogContext {
            state: &mut state,
            env: &envelope,
        };
        calls_post(&deps, &call, &mut log).await
    });
    assert!(matches!(answer, Err(RealtimeHttpError::Sdp(_))));
    assert!(input_untouched(&state));
    assert!(!state.model_call_details.contains_key("additional_args"));
}

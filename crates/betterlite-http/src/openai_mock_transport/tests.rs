#![cfg(test)]

use super::{
    DecodeMockRequestError, TransportChoice, decode_mock_request, handle_mock_request,
    select_transport,
};
use rstest::rstest;
use std::fs;
use uuid::Uuid;

#[derive(serde::Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
enum DecodeExpectation {
    Model { model_json: String },
    Error { type_name: String },
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DecoderProbe {
    content: String,
    expected: DecodeExpectation,
}

#[derive(serde::Deserialize)]
#[serde(tag = "case", rename_all = "snake_case", deny_unknown_fields)]
enum Scenario {
    ReplyLiterals {
        content: String,
        clock: i64,
        uuid: String,
        expected_model_json: String,
    },
    RequestDecoder {
        probes: Vec<DecoderProbe>,
    },
    Selection,
}

fn assert_reply_literals(content: &str, clock: i64, uuid: &str, expected_model_json: &str) {
    let identifier = Uuid::parse_str(uuid).unwrap();
    let clock_fn = || clock;
    let identifier_fn = || identifier;
    let response = handle_mock_request(content.as_bytes(), &clock_fn, &identifier_fn).unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json");
    let body = response.body;
    assert_eq!(body.object, "chat.completion");
    assert_eq!(body.created, clock);
    assert_eq!(
        serde_json::to_string(&body.model).unwrap(),
        expected_model_json
    );
    let short_id = identifier
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect::<String>();
    assert_eq!(body.id, format!("chatcmpl-mock-{short_id}"));
    let choice = body.choices.first().unwrap();
    assert_eq!(choice.index, 0);
    assert_eq!(choice.message.role, "assistant");
    assert_eq!(choice.message.content, "Mock response");
    assert_eq!(choice.finish_reason, "stop");
    assert_eq!(body.usage.prompt_tokens, 1);
    assert_eq!(body.usage.completion_tokens, 1);
    assert_eq!(body.usage.total_tokens, 2);
}

fn assert_decoder_probe(probe: DecoderProbe) {
    let outcome = decode_mock_request(probe.content.as_bytes());
    match probe.expected {
        DecodeExpectation::Model { model_json } => {
            let decoded = outcome.unwrap();
            assert!(!decoded.stream);
            assert_eq!(serde_json::to_string(&decoded.model).unwrap(), model_json);
        }
        DecodeExpectation::Error { type_name } => {
            assert_eq!(
                outcome.unwrap_err(),
                DecodeMockRequestError::NotAMapping { type_name }
            );
        }
    }
}

const fn expected_unsupplied(network_mock: bool) -> TransportChoice<&'static str> {
    if network_mock {
        TransportChoice::Mock
    } else {
        TransportChoice::Default
    }
}

fn assert_selection_case(network_mock: bool, shared_session: Option<&str>) {
    let async_supplied = select_transport(Some("async-client"), network_mock, shared_session);
    assert_eq!(async_supplied, TransportChoice::Supplied("async-client"));
    let sync_supplied = select_transport(Some("sync-client"), network_mock, shared_session);
    assert_eq!(sync_supplied, TransportChoice::Supplied("sync-client"));
    let unsupplied: TransportChoice<&str> = select_transport(None, network_mock, shared_session);
    assert_eq!(unsupplied, expected_unsupplied(network_mock));
}

fn assert_selection() {
    for network_mock in [false, true] {
        for shared_session in [None, Some("shared-session")] {
            assert_selection_case(network_mock, shared_session);
        }
    }
}

fn run_scenario(source: &str) {
    match serde_json::from_str(source).unwrap() {
        Scenario::ReplyLiterals {
            content,
            clock,
            uuid,
            expected_model_json,
        } => assert_reply_literals(&content, clock, &uuid, &expected_model_json),
        Scenario::RequestDecoder { probes } => {
            for probe in probes {
                assert_decoder_probe(probe);
            }
        }
        Scenario::Selection => assert_selection(),
    }
}

#[rstest]
#[case::mock_reply_literals(
    r#"{"case":"reply_literals","content":"{\"model\":\"gpt-4\",\"stream\":true}","clock":1700000000,"uuid":"01234567-89ab-cdef-0123-456789abcdef","expected_model_json":"\"gpt-4\""}"#
)]
#[case::mock_request_decoder(
    r#"{"case":"request_decoder","probes":[
        {"content":"not json","expected":{"outcome":"model","model_json":"\"mock-model\""}},
        {"content":"{\"model\": \"gpt-4\"} extra","expected":{"outcome":"model","model_json":"\"mock-model\""}},
        {"content":"{\"model\": \"gpt-4\"}","expected":{"outcome":"model","model_json":"\"gpt-4\""}},
        {"content":"{\"model\": null}","expected":{"outcome":"model","model_json":"null"}},
        {"content":"{\"model\": \"\"}","expected":{"outcome":"model","model_json":"\"\""}},
        {"content":"{\"model\": 7}","expected":{"outcome":"model","model_json":"7"}},
        {"content":"{\"other\": 1}","expected":{"outcome":"model","model_json":"\"mock-model\""}},
        {"content":"[1,2,3]","expected":{"outcome":"error","type_name":"list"}},
        {"content":"\"just a string\"","expected":{"outcome":"error","type_name":"str"}},
        {"content":"42","expected":{"outcome":"error","type_name":"int"}},
        {"content":"true","expected":{"outcome":"error","type_name":"bool"}},
        {"content":"null","expected":{"outcome":"error","type_name":"NoneType"}}
    ]}"#
)]
#[case::mock_selection(r#"{"case":"selection"}"#)]
fn openai_mock_transport_cases(#[case] source: &str) {
    run_scenario(source);
}

#[test]
fn replays_the_openai_mock_transport_corpus() {
    let entries = fs::read_dir("fuzz/corpus/openai_mock_transport").unwrap();
    let mut seen = 0_usize;
    for entry in entries {
        let path = entry.unwrap().path();
        let content = fs::read(&path).unwrap();
        match decode_mock_request(&content) {
            Ok(decoded) => assert!(!decoded.stream),
            Err(DecodeMockRequestError::NotAMapping { type_name }) => {
                assert!(!type_name.is_empty());
            }
        }
        seen += 1;
    }
    assert!(seen > 0_usize);
}

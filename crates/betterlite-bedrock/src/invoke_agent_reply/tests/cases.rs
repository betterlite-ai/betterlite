#![cfg(test)]

use betterlite_models::decisions::Json;
use bytes::BytesMut;
use rstest::rstest;

use crate::bedrock_settings::ErrorHeaders;
use crate::event_frames::decode_aws_event_frame;
use crate::event_frames::error::EventFrameError;
use crate::event_frames::message::parse_message_from_event;
use crate::event_frames::shape::ResponseStreamShape;
use crate::invoke_agent_reply::decode_agent_trace;
use crate::invoke_agent_reply::error::InvokeAgentReplyError;
use crate::invoke_agent_reply::events::{AgentEvent, EventHeaders};
use crate::invoke_agent_reply::extract::{ExtractError, UsageInfo, response_content, usage_info};
use crate::invoke_agent_reply::tests::support::{
    Header, bedrock_status, capture, chunk, chunk_body, completed, content, event_frame, events,
    frame, join, json, run, supplied, trace, usage,
};

const USAGE_HEAD: &str =
    r#"{"trace":{"preProcessingTrace":{"modelInvocationOutput":{"metadata":{"usage":"#;
const USAGE_TAIL: &str = "}}}}}}";
const MODEL_HEAD: &str =
    r#"{"trace":{"orchestrationTrace":{"modelInvocationInput":{"foundationModel":"#;
const MODEL_TAIL: &str = "}}}}";

#[rstest]
#[case::agent_event_stream_content(agent_event_stream_content)]
#[case::chunk_parse_and_decode(chunk_parse_and_decode)]
#[case::agent_error_frame(agent_error_frame)]
#[case::trace_json_warning(trace_json_warning)]
#[case::preprocess_trace_usage(preprocess_trace_usage)]
#[case::orchestration_model(orchestration_model)]
#[case::agent_reply_and_failure(agent_reply_and_failure)]
fn invoke_agent_reply_cases(#[case] case: fn()) {
    case();
}

fn usage_trace(usage: &str) -> Vec<u8> {
    trace(&format!("{USAGE_HEAD}{usage}{USAGE_TAIL}"))
}

fn model_trace(model: &str) -> Vec<u8> {
    trace(&format!("{MODEL_HEAD}{model}{MODEL_TAIL}"))
}

fn chunk_event(payload: Json) -> AgentEvent {
    AgentEvent {
        headers: EventHeaders {
            event_type: "chunk".to_owned(),
            ..EventHeaders::default()
        },
        payload,
    }
}

fn agent_event_stream_content() {
    let stream = join(&[
        event_frame("mystery", "event", b"{\"bytes\":\"aGk=\"}"),
        event_frame("weird", "exception", b"{\"message\":\"bad\"}"),
        chunk("Hello"),
        usage_trace(r#"{"inputTokens":2,"outputTokens":1}"#),
        chunk(" world"),
    ]);
    let response = completed(200, &stream);
    assert_eq!(content(&response), "Hello world");
    assert_eq!(usage(&response), (2, 1, 3));

    let recognized = events(&stream);
    assert_eq!(recognized.len(), 3);
    assert_eq!(recognized[0].headers.event_type, "chunk");
    assert_eq!(recognized[0].headers.content_type, "application/json");
    assert_eq!(recognized[0].headers.message_type, "event");
    assert_eq!(recognized[1].headers.event_type, "trace");

    let bare = frame(&[], b"{\"bytes\":\"aGk=\"}");
    assert!(events(&bare).is_empty());
    let only_event_type = frame(
        &[Header::Text(":event-type", "chunk")],
        b"{\"bytes\":\"aGk=\"}",
    );
    let defaulted = events(&only_event_type);
    assert_eq!(defaulted.len(), 1);
    assert_eq!(defaulted[0].headers.content_type, "");
    assert_eq!(defaulted[0].headers.message_type, "");

    let (captured, guard) = capture();
    let integer_type = frame(
        &[
            Header::Integer(":event-type", 5),
            Header::Text(":content-type", "application/json"),
            Header::Text(":message-type", "event"),
        ],
        b"{\"bytes\":\"aGk=\"}",
    );
    assert!(events(&integer_type).is_empty());
    let logs = captured.text();
    assert!(logs.contains("Error extracting headers:"), "{logs}");
    assert!(logs.contains("Unknown event type: "), "{logs}");
    drop(guard);

    for event_type in ["", "Chunk", "chunks"] {
        let unknown = event_frame(event_type, "event", b"{\"bytes\":\"aGk=\"}");
        assert!(events(&unknown).is_empty(), "{event_type}");
    }
}

fn chunk_parse_and_decode() {
    assert_eq!(
        content(&completed(200, &join(&[chunk("Hel"), chunk("lo")]))),
        "Hello"
    );
    for body in [
        "{}",
        "{\"bytes\":null}",
        "{\"bytes\":\"\"}",
        "{\"bytes\":\"!!!\"}",
        "{\"bytes\":\"/w==\"}",
        "{\"bytes\":5}",
        "not json",
        "\"text\"",
    ] {
        assert_eq!(content(&completed(200, &chunk_body(body))), "", "{body}");
    }

    let decoded = events(&chunk("hi"));
    assert_eq!(decoded[0].payload, json("{\"bytes\":\"aGk=\"}"));
    assert!(events(&trace("")).is_empty());

    let (captured, guard) = capture();
    let broken_parse = events(&join(&[
        chunk_body("{\"bytes\":\"!!!\"}"),
        chunk_body("not json"),
    ]));
    assert!(broken_parse.is_empty());
    let logs = captured.text();
    assert!(logs.contains("Error parsing message from event:"), "{logs}");
    drop(guard);

    let direct = [
        chunk_event(json("{\"bytes\":\"aGk=\"}")),
        chunk_event(json("{\"bytes\":\"aGk=\"}")),
    ];
    assert_eq!(response_content(&direct).unwrap(), "hihi");

    let (captured, guard) = capture();
    let skipped = [
        chunk_event(json("{\"bytes\":\"!!!\"}")),
        chunk_event(json("{\"bytes\":\"/w==\"}")),
        chunk_event(json("{\"bytes\":5}")),
        chunk_event(json("{\"bytes\":\"\"}")),
        chunk_event(json("null")),
        chunk_event(json("{}")),
    ];
    assert_eq!(response_content(&skipped).unwrap(), "");
    let logs = captured.text();
    assert!(logs.contains("Failed to decode chunk content:"), "{logs}");
    drop(guard);

    let empty_body = chunk_event(json("{}"));
    assert_eq!(response_content(&[empty_body]).unwrap(), "");
    for payload in ["[1]", "\"chunk\"", "5", "true"] {
        let chunk_events = [chunk_event(json(payload))];
        let error = response_content(&chunk_events).unwrap_err();
        assert!(
            matches!(error, ExtractError::NotObject { field } if field == "chunk payload"),
            "{payload}"
        );
    }
}

fn agent_error_frame() {
    let exception = event_frame("chunk", "exception", b"{\"message\":\"bad\"}");
    let stream = join(&[exception.clone(), chunk("keep")]);
    let (captured, guard) = capture();
    let response = completed(200, &stream);
    let logs = captured.text();
    drop(guard);
    assert_eq!(content(&response), "keep");
    assert_eq!(events(&stream).len(), 1);
    assert!(logs.contains("Error parsing message from event:"), "{logs}");

    let trace_error = event_frame("trace", "error", b"{\"message\":\"bad\"}");
    assert_eq!(content(&completed(200, &trace_error)), "");
    let invalid = event_frame("chunk", "exception", b"boom");
    assert_eq!(content(&completed(200, &invalid)), "");

    let mut buffer = BytesMut::from(exception.as_slice());
    let single = decode_aws_event_frame(&mut buffer).unwrap().unwrap();
    let error = parse_message_from_event(Some(&ResponseStreamShape::bedrock_runtime()), &single)
        .unwrap_err();
    match error {
        EventFrameError::Bedrock { status, error } => {
            assert_eq!(status, 400);
            assert_eq!(
                error.error.message,
                "validationException {\"message\":\"bad\"}"
            );
        }
        other => panic!("expected a bedrock frame error, got {other:?}"),
    }

    let mut broken_crc = chunk("ok");
    *broken_crc.last_mut().unwrap() ^= 0xFF;
    assert!(decode_agent_trace(&broken_crc).is_err());
    assert_eq!(bedrock_status(run(200, &broken_crc).0), 200);

    let mut broken_prelude = chunk("ok");
    *broken_prelude.get_mut(8).unwrap() ^= 0xFF;
    assert!(matches!(
        decode_agent_trace(&broken_prelude).unwrap_err(),
        EventFrameError::PreludeChecksum { .. }
    ));

    let truncated = join(&[chunk("ok"), chunk("cut")[..20].to_vec()]);
    assert_eq!(content(&completed(200, &truncated)), "ok");
    assert!(events(&[0_u8; 5]).is_empty());
}

fn trace_json_warning() {
    let invalid = join(&[trace("{oops"), chunk("ok")]);
    let (captured, guard) = capture();
    let response = completed(200, &invalid);
    let logs = captured.text();
    drop(guard);
    assert_eq!(content(&response), "ok");
    assert_eq!(usage(&response), (0, 0, 0));
    assert!(logs.contains("Failed to parse trace event JSON:"), "{logs}");

    for body in ["null", "false", "0", "\"\"", "[]", "{}"] {
        let decoded = events(&trace(body));
        assert_eq!(decoded.len(), 1, "{body}");
        assert_eq!(decoded[0].payload, json(body));
        assert_eq!(usage(&completed(200, &trace(body))), (0, 0, 0), "{body}");
    }

    for body in ["[1,2]", "\"hello\"", "1", "true"] {
        let decoded = events(&trace(body));
        let error = usage_info(&decoded).unwrap_err();
        assert!(
            matches!(error, ExtractError::NotObject { field } if field == "trace payload"),
            "{body}"
        );
        assert_eq!(bedrock_status(run(200, &trace(body)).0), 200);
    }

    let nested = trace("{\"trace\":[1]}");
    assert!(matches!(
        usage_info(&events(&nested)).unwrap_err(),
        ExtractError::NotObject { field } if field == "trace"
    ));
}

fn preprocess_trace_usage() {
    let single = usage_info(&events(&usage_trace(
        r#"{"inputTokens":10,"outputTokens":5}"#,
    )))
    .unwrap();
    assert_eq!((single.input_tokens, single.output_tokens), (10, 5));

    let summed = usage_info(&events(&join(&[
        usage_trace(r#"{"inputTokens":3,"outputTokens":1}"#),
        usage_trace(r#"{"inputTokens":4,"outputTokens":2}"#),
    ])))
    .unwrap();
    assert_eq!((summed.input_tokens, summed.output_tokens), (7, 3));

    let missing_input = usage_info(&events(&usage_trace(r#"{"outputTokens":5}"#))).unwrap();
    assert_eq!(
        (missing_input.input_tokens, missing_input.output_tokens),
        (0, 5)
    );

    for body in [
        usage_trace("{}"),
        trace(r#"{"trace":{"preProcessingTrace":{"modelInvocationOutput":{"metadata":{}}}}}"#),
        trace(r#"{"trace":{"preProcessingTrace":{"modelInvocationOutput":{}}}}"#),
        trace(r#"{"trace":{"preProcessingTrace":{}}}"#),
        trace(r#"{"trace":{}}"#),
        trace("{}"),
    ] {
        assert_eq!(usage_info(&events(&body)).unwrap(), UsageInfo::default());
    }

    let orchestration_only = usage_info(&events(&trace(
        r#"{"trace":{"orchestrationTrace":{"modelInvocationOutput":{"metadata":{"usage":{"inputTokens":8,"outputTokens":9}}}}}}"#,
    )))
    .unwrap();
    assert_eq!(
        (
            orchestration_only.input_tokens,
            orchestration_only.output_tokens
        ),
        (0, 0)
    );

    for counts in ["{\"inputTokens\":\"5\"}", "{\"inputTokens\":null}"] {
        let body = usage_trace(counts);
        let error = usage_info(&events(&body)).unwrap_err();
        assert!(
            matches!(error, ExtractError::NotInteger { field } if field == "inputTokens"),
            "{counts}"
        );
        assert_eq!(bedrock_status(run(200, &body).0), 200);
    }

    let nonobject_usage = trace(&format!("{USAGE_HEAD}[1]{USAGE_TAIL}"));
    assert!(matches!(
        usage_info(&events(&nonobject_usage)).unwrap_err(),
        ExtractError::NotObject { field } if field == "usage"
    ));

    let huge = usage_trace(&format!("{{\"inputTokens\":{}}}", i64::MAX));
    assert!(matches!(
        usage_info(&events(&join(&[huge, usage_trace("{\"inputTokens\":1}")]))).unwrap_err(),
        ExtractError::Overflow
    ));

    let total_overflow = usage_trace(&format!(
        "{{\"inputTokens\":{},\"outputTokens\":1}}",
        i64::MAX
    ));
    assert_eq!(bedrock_status(run(200, &total_overflow).0), 200);
}

fn orchestration_model() {
    let selected = |body: &[u8]| usage_info(&events(body)).map(|usage| usage.model);
    assert_eq!(
        selected(&model_trace("\"anthropic.claude-v2\"")).unwrap(),
        Some("anthropic.claude-v2".to_owned())
    );
    assert_eq!(selected(&model_trace("\"\"")).unwrap(), Some(String::new()));
    assert_eq!(selected(&model_trace("null")).unwrap(), None);

    let second = join(&[trace("{\"trace\":{}}"), model_trace("\"m2\"")]);
    assert_eq!(selected(&second).unwrap(), Some("m2".to_owned()));

    let after_null = join(&[model_trace("null"), model_trace("\"m2\"")]);
    assert_eq!(selected(&after_null).unwrap(), Some("m2".to_owned()));

    let after_empty = join(&[model_trace("\"\""), model_trace("\"m2\"")]);
    assert_eq!(selected(&after_empty).unwrap(), Some(String::new()));

    let first_wins = join(&[model_trace("\"m1\""), model_trace("\"m2\"")]);
    assert_eq!(selected(&first_wins).unwrap(), Some("m1".to_owned()));

    for body in [
        trace("{\"trace\":{}}"),
        trace("{\"trace\":{\"orchestrationTrace\":{}}}"),
        trace("{\"trace\":{\"orchestrationTrace\":{\"modelInvocationInput\":{}}}}"),
    ] {
        assert_eq!(selected(&body).unwrap(), None);
        assert_eq!(completed(200, &body).model, None);
    }

    let wrong = model_trace("5");
    assert!(matches!(
        selected(&wrong).unwrap_err(),
        ExtractError::NotText { field } if field == "foundationModel"
    ));
    assert_eq!(bedrock_status(run(200, &wrong).0), 200);

    assert_eq!(
        completed(200, &model_trace("\"\"")).model,
        Some(String::new())
    );
}

fn agent_reply_and_failure() {
    let stream = join(&[
        chunk("Hello"),
        usage_trace(r#"{"inputTokens":2,"outputTokens":3}"#),
    ]);
    let response = completed(200, &stream);
    assert_eq!(response.id, "chatcmpl-kept");
    assert_eq!(response.object, "chat.completion");
    assert_eq!(response.created, 7);
    assert_eq!(response.system_fingerprint.as_deref(), Some("fp-1"));
    assert_eq!(response.service_tier, Some(json("\"priority\"")));
    assert_eq!(response.trace, Some(json("{\"guardrail\":\"kept\"}")));
    assert_eq!(response.model, None);
    let choice = response.choices.first().unwrap();
    assert_eq!(choice.index, 0);
    assert_eq!(choice.finish_reason, "stop");
    assert_eq!(choice.message.role, "assistant");
    assert_eq!(choice.message.content.as_deref(), Some("Hello"));
    assert_eq!(usage(&response), (2, 3, 5));

    let empty = completed(200, b"");
    assert_eq!(content(&empty), "");
    assert_eq!(usage(&empty), (0, 0, 0));
    assert_eq!(empty.choices.len(), 1);

    let mut broken = chunk("ok");
    *broken.last_mut().unwrap() ^= 0xFF;
    let (captured, guard) = capture();
    let (outcome, untouched) = run(200, &broken);
    let logs = captured.text();
    drop(guard);
    assert!(logs.contains("Error processing Bedrock Invoke Agent response:"));
    assert_eq!(untouched, supplied());
    match outcome {
        Err(InvokeAgentReplyError::Bedrock { status, error }) => {
            assert_eq!(status, 200);
            assert_eq!(error.cause.error.status_code, 200);
            assert!(
                error
                    .cause
                    .error
                    .message
                    .starts_with("Error processing response: ")
            );
            let headers = vec![(
                "content-type".to_owned(),
                "application/vnd.amazon.eventstream".to_owned(),
            )];
            assert_eq!(
                error.cause.headers,
                Some(ErrorHeaders::Http(headers.clone()))
            );
            assert_eq!(error.cause.error.headers, Some(headers));
        }
        other => panic!("expected a bedrock failure, got {other:?}"),
    }

    assert_eq!(content(&completed(500, &chunk("ok"))), "ok");
    let truncated = join(&[chunk("ok"), chunk("cut")[..20].to_vec()]);
    assert_eq!(content(&completed(503, &truncated)), "ok");
    assert_eq!(bedrock_status(run(500, &broken).0), 500);

    let mut broken_prelude = chunk("ok");
    *broken_prelude.get_mut(8).unwrap() ^= 0xFF;
    assert_eq!(bedrock_status(run(200, &broken_prelude).0), 200);
    assert_eq!(bedrock_status(run(502, &broken_prelude).0), 502);
}

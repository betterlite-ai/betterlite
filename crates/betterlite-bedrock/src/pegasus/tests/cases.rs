use betterlite_tokenizer::count::request::Message;
use rstest::rstest;
use serde_json::from_str;

use crate::bedrock_settings::ErrorHeaders;
use crate::image_models::params::{Field, Params};
use crate::pegasus::error::PegasusError;
use crate::pegasus::media::build_media_source;
use crate::pegasus::options::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use crate::pegasus::prompt::{PromptMessage, convert_messages_to_prompt};
use crate::pegasus::reply::decode_pegasus_reply;
use crate::pegasus::request::transform_request;
use crate::pegasus::response::PegasusResponse;
use crate::pegasus::response_format::normalize_response_format;

use super::support::{
    CounterCall, FixtureCounter, block_on, choice, context, header_counts, response, single_choice,
};

#[rstest]
#[case::pegasus_media_source(pegasus_media_source)]
#[case::text_video_prompt(text_video_prompt)]
#[case::response_format_json(response_format_json)]
#[case::segments_and_response(segments_and_response)]
#[case::usage_and_finish_reason(usage_and_finish_reason)]
#[case::pegasus_options(pegasus_options)]
#[case::pegasus_reply_errors(pegasus_reply_errors)]
fn pegasus_cases(#[case] check: fn()) {
    check();
}

fn params(json: &str) -> Params {
    from_str(json).unwrap()
}

fn field(json: &str) -> Field {
    from_str(json).unwrap()
}

fn prompt_messages(json: &str) -> Vec<PromptMessage> {
    from_str(json).unwrap()
}

fn messages(json: &str) -> Vec<Message> {
    from_str(json).unwrap()
}

const DIRECT_MEDIA: &[(&str, Option<&str>)] = &[
    (
        r#"{"mediaSource":{"s3Location":{"uri":"s3://b/k"}}}"#,
        Some(r#"{"s3Location":{"uri":"s3://b/k"}}"#),
    ),
    (r#"{"media_source":{"custom":1}}"#, Some(r#"{"custom":1}"#)),
    (r#"{"mediaSource":"","media_source":{}}"#, Some("{}")),
    (
        r#"{"mediaSource":"literal","media_source":{"a":"b"}}"#,
        None,
    ),
    (
        r#"{"mediaSource":0,"media_source":{"a":"b"}}"#,
        Some(r#"{"a":"b"}"#),
    ),
];

const INLINE_MEDIA: &[(&str, Option<&str>)] = &[
    (
        r#"{"video_base64":"data:video/mp4;base64,QUJD"}"#,
        Some(r#"{"base64String":"QUJD"}"#),
    ),
    (
        r#"{"base64_string":"QUJD"}"#,
        Some(r#"{"base64String":"QUJD"}"#),
    ),
    (
        r#"{"mediaSource":"literal","video_base64":"QUJD"}"#,
        Some(r#"{"base64String":"QUJD"}"#),
    ),
    (
        r#"{"video_base64":"prefix,first,second"}"#,
        Some(r#"{"base64String":"first"}"#),
    ),
    (
        r#"{"video_base64":"QVxB","base64_string":"QUJD"}"#,
        Some(r#"{"base64String":"QVxB"}"#),
    ),
];

const S3_MEDIA: &[(&str, Option<&str>)] = &[
    (
        r#"{"s3_uri":"s3://b"}"#,
        Some(r#"{"s3Location":{"uri":"s3://b"}}"#),
    ),
    (
        r#"{"video_s3_uri":"s3://a","video_s3_bucket_owner":"own-a"}"#,
        Some(r#"{"s3Location":{"uri":"s3://a","bucketOwner":"own-a"}}"#),
    ),
    (
        r#"{"mediaSource":"literal","video_s3_uri":"s3://a"}"#,
        Some(r#"{"s3Location":{"uri":"s3://a"}}"#),
    ),
    (
        r#"{"video_s3_uri":"s3://a","s3_uri":"s3://b","s3_bucket_owner":"own-b"}"#,
        Some(r#"{"s3Location":{"uri":"s3://a","bucketOwner":"own-b"}}"#),
    ),
    (
        r#"{"media_source_s3_uri":"s3://c","video_s3_bucket_owner":"","media_source_bucket_owner":"own-c"}"#,
        Some(r#"{"s3Location":{"uri":"s3://c","bucketOwner":"own-c"}}"#),
    ),
    ("{}", None),
];

fn pegasus_media_source() {
    for &(json, expected) in DIRECT_MEDIA.iter().chain(INLINE_MEDIA).chain(S3_MEDIA) {
        assert_eq!(media_source(json), expected.map(field), "{json}");
    }
    assert!(matches!(
        build_media_source(&params(r#"{"video_base64":5}"#)),
        Err(PegasusError::NotText {
            field: "video_base64"
        })
    ));
    assert!(matches!(
        build_media_source(&params(r#"{"video_base64":"","base64_string":{"a":1}}"#)),
        Err(PegasusError::NotText {
            field: "base64_string"
        })
    ));
}

fn media_source(json: &str) -> Option<Field> {
    build_media_source(&params(json)).unwrap()
}

fn text_video_prompt() {
    let prompt = |json: &str| convert_messages_to_prompt(&prompt_messages(json)).unwrap();
    assert_eq!(
        prompt(r#"[{"role":"system","content":"be brief"},{"content":"hi"}]"#),
        "system: be brief\nuser: hi"
    );
    assert_eq!(prompt(r#"[{"role":"user"}]"#), "user:");
    assert_eq!(prompt(r#"[{"role":"user","content":null}]"#), "user: None");
    assert_eq!(prompt(r#"[{"role":null,"content":"x"}]"#), "None: x");
    assert_eq!(prompt("[]"), "");
    assert_eq!(
        prompt(r#"[{"role":"user","content":"  padded  "}]"#),
        "user:   padded"
    );
    assert_eq!(
        prompt(
            r#"[{"content":["a",{"type":"text","text":"b"},{"type":"image_url"},{"type":"video_url"},{"type":"audio_url"},{"type":"tool_use"},5]}]"#
        ),
        "user: a b <image> <video> <audio>"
    );
    assert_eq!(
        prompt(r#"[{"content":["a",{"type":"text"},"b"]}]"#),
        "user: a  b"
    );
    assert_eq!(
        prompt(r#"[{"content":[{"type":"text","text":null},"a"]}]"#),
        "user:  a"
    );
}

fn response_format_json() {
    let normalize = |json: &str| normalize_response_format(&field(json));
    assert_eq!(
        normalize(
            r#"{"type":"json_schema","json_schema":{"name":"x","schema":{"type":"object"}}}"#
        )
        .unwrap(),
        field(r#"{"jsonSchema":{"type":"object"}}"#)
    );
    assert_eq!(
        normalize(r#"{"json_schema":{"name":"x"}}"#).unwrap(),
        field(r#"{"jsonSchema":{"name":"x"}}"#)
    );
    assert_eq!(
        normalize(r#"{"jsonSchema":{"a":1},"json_schema":{"schema":{"b":2}}}"#).unwrap(),
        field(r#"{"jsonSchema":{"b":2}}"#)
    );
    assert_eq!(normalize("{}").unwrap(), field("{}"));
    assert_eq!(normalize("null").unwrap(), Field::Null);
    assert!(matches!(
        normalize(r#""json_object""#),
        Err(PegasusError::UnsupportedResponseFormat)
    ));
    let mapped = map_openai_params(
        &params(r#"{"response_format":{"json_schema":{"schema":{"type":"object"}}}}"#),
        Params::new(),
    )
    .unwrap();
    assert_eq!(
        mapped.get("responseFormat"),
        Some(&field(r#"{"jsonSchema":{"type":"object"}}"#))
    );
}

fn segments_and_response() {
    let headers = header_counts("3", "5");
    let given = messages(r#"[{"role":"user","content":"hi"}]"#);
    let mut full = response(vec![
        choice(0, None, "stop"),
        choice(1, Some("kept"), "length"),
    ]);
    block_on(decode_pegasus_reply(
        br#"{"message":"visible","finishReason":"tool_use","segments":[{"text":"x"}],"response":"alias","usage":{"prompt_tokens":99}}"#,
        &context(200, &headers, &given, 10.0),
        &mut full,
        &FixtureCounter::counting(0, 0),
    ))
    .unwrap();
    assert_eq!(full.choices[0].message.content.as_deref(), Some("visible"));
    assert_eq!(full.choices[0].finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(full.choices[1], choice(1, Some("kept"), "length"));
    assert_eq!(full.id, "chatcmpl-pegasus");
    assert_eq!((full.usage.prompt_tokens, full.usage.total_tokens), (3, 8));
    let mut missing = single_choice();
    block_on(decode_pegasus_reply(
        br#"{"message":"visible"}"#,
        &context(200, &headers, &given, 10.0),
        &mut missing,
        &FixtureCounter::counting(0, 0),
    ))
    .unwrap();
    assert_eq!(missing.choices[0].finish_reason.as_deref(), Some("stop"));
}

fn usage_and_finish_reason() {
    header_counts_win();
    counted_usage();
    mixed_case_headers();
}

fn header_counts_win() {
    let given = messages(r#"[{"role":"user","content":"hi"}]"#);
    let fixture = FixtureCounter::counting(100, 200);
    let mut counted = single_choice();
    block_on(decode_pegasus_reply(
        br#"{"message":"visible"}"#,
        &context(200, &header_counts("0", "-3"), &given, 1_700_000_000.9),
        &mut counted,
        &fixture,
    ))
    .unwrap();
    assert_eq!(counted.usage.prompt_tokens, 0);
    assert_eq!(counted.usage.completion_tokens, -3);
    assert_eq!(counted.usage.total_tokens, -3);
    assert_eq!(counted.created, 1_700_000_000);
    assert_eq!(counted.model, "twelvelabs.pegasus-1-2-v1:0");
    assert!(fixture.calls().is_empty());
}

fn counted_usage() {
    let given = messages(r#"[{"role":"user","content":"hi"}]"#);
    let fallback = FixtureCounter::counting(11, 7);
    let mut counted_fallback = single_choice();
    block_on(decode_pegasus_reply(
        br#"{"message":"visible"}"#,
        &context(200, &[], &given, 5.5),
        &mut counted_fallback,
        &fallback,
    ))
    .unwrap();
    assert_eq!(
        fallback.calls(),
        vec![
            CounterCall::Messages,
            CounterCall::Text("visible".to_owned())
        ]
    );
    assert_eq!(counted_fallback.usage.total_tokens, 18);
    let mut empty_headers = single_choice();
    block_on(decode_pegasus_reply(
        br#"{"message":"visible"}"#,
        &context(200, &header_counts("", ""), &given, 5.5),
        &mut empty_headers,
        &FixtureCounter::counting(11, 7),
    ))
    .unwrap();
    assert_eq!(empty_headers.usage.total_tokens, 18);
}

fn mixed_case_headers() {
    let given = messages(r#"[{"role":"user","content":"hi"}]"#);
    let mixed = vec![
        (
            "X-Amzn-Bedrock-Input-Token-Count".to_owned(),
            "3".to_owned(),
        ),
        (
            "X-Amzn-Bedrock-Output-Token-Count".to_owned(),
            "5".to_owned(),
        ),
    ];
    let mut mixed_case = single_choice();
    block_on(decode_pegasus_reply(
        br#"{"message":"visible"}"#,
        &context(200, &mixed, &given, 5.5),
        &mut mixed_case,
        &FixtureCounter::counting(11, 7),
    ))
    .unwrap();
    assert_eq!(
        (
            mixed_case.usage.prompt_tokens,
            mixed_case.usage.completion_tokens
        ),
        (3, 5)
    );
    assert_eq!(mixed_case.usage.total_tokens, 8);
}

fn pegasus_options() {
    supported_and_mapped();
    request_keeps_options();
    response_format_requests();
}

fn supported_and_mapped() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "max_tokens",
            "max_completion_tokens",
            "temperature",
            "response_format"
        ]
    );
    let mapped = map_openai_params(
        &params(r#"{"max_tokens":10,"temperature":0.5,"top_p":0.9,"max_completion_tokens":20}"#),
        params(r#"{"custom":"keep"}"#),
    )
    .unwrap();
    assert_eq!(
        mapped,
        params(r#"{"custom":"keep","maxOutputTokens":20,"temperature":0.5}"#)
    );
    let nulls = map_openai_params(
        &params(r#"{"temperature":null,"max_tokens":null}"#),
        Params::new(),
    )
    .unwrap();
    assert_eq!(
        nulls,
        params(r#"{"maxOutputTokens":null,"temperature":null}"#)
    );
}

fn request_keeps_options() {
    let optional = params(
        r#"{"mediaSource":{"base64String":"QUJD"},"temperature":null,"responseFormat":{},"top_p":0.9}"#,
    );
    let request =
        transform_request(&prompt_messages(r#"[{"content":"hello"}]"#), &optional).unwrap();
    assert_eq!(
        request,
        params(
            r#"{"inputPrompt":"user: hello","mediaSource":{"base64String":"QUJD"},"temperature":null}"#
        )
    );
    assert_eq!(
        optional,
        params(
            r#"{"mediaSource":{"base64String":"QUJD"},"temperature":null,"responseFormat":{},"top_p":0.9}"#
        )
    );
}

fn response_format_requests() {
    let truthy = transform_request(
        &prompt_messages("[]"),
        &params(r#"{"responseFormat":{"jsonSchema":{"type":"object"}}}"#),
    )
    .unwrap();
    assert_eq!(
        truthy,
        params(r#"{"inputPrompt":"","responseFormat":{"jsonSchema":{"type":"object"}}}"#)
    );
    let raw_schema = transform_request(
        &prompt_messages("[]"),
        &params(r#"{"responseFormat":{"json_schema":{"schema":{"type":"object"}}}}"#),
    )
    .unwrap();
    assert_eq!(
        raw_schema,
        params(r#"{"inputPrompt":"","responseFormat":{"jsonSchema":{"type":"object"}}}"#)
    );
}

fn pegasus_reply_errors() {
    parse_errors();
    reply_shape_errors();
    content_errors();
    content_catch_error();
    count_errors();
}

fn decode_body(
    body: &[u8],
    response: &mut PegasusResponse,
    headers: &[(String, String)],
    given: &[Message],
) -> Result<(), PegasusError> {
    block_on(decode_pegasus_reply(
        body,
        &context(200, headers, given, 10.0),
        response,
        &FixtureCounter::counting(0, 0),
    ))
}

fn parse_errors() {
    let headers = header_counts("1", "2");
    let given = messages("[]");
    let mut response = single_choice();
    let failure = decode_body(b"{not json", &mut response, &headers, &given).unwrap_err();
    let PegasusError::Bedrock { status, error } = failure else {
        panic!("expected a bedrock error, got {failure:?}");
    };
    assert_eq!(status, 200);
    assert!(matches!(&error.cause.headers, Some(ErrorHeaders::Http(_))));
    assert!(error.cause.error.message.contains("Error parsing response"));
    let not_utf8 = decode_body(&[0xff, 0xfe], &mut response, &headers, &given).unwrap_err();
    let PegasusError::Bedrock {
        status: utf8_status,
        error: utf8_error,
    } = not_utf8
    else {
        panic!("expected a bedrock error, got {not_utf8:?}");
    };
    assert_eq!(utf8_status, 200);
    assert!(
        utf8_error
            .cause
            .error
            .message
            .contains("Error parsing response: the body is not UTF-8")
    );
}

fn reply_shape_errors() {
    let headers = header_counts("1", "2");
    let given = messages("[]");
    let mut response = single_choice();
    assert!(matches!(
        decode_body(b"[1,2]", &mut response, &headers, &given),
        Err(PegasusError::ReplyNotObject)
    ));
    assert!(matches!(
        decode_body(
            br#"{"message":"visible","finishReason":[]}"#,
            &mut response,
            &headers,
            &given
        ),
        Err(PegasusError::FinishReasonNotMappable)
    ));
    assert!(matches!(
        decode_body(
            br#"{"message":{"a":1},"finishReason":"stop"}"#,
            &mut response,
            &headers,
            &given
        ),
        Err(PegasusError::MessageNotString)
    ));
    assert!(matches!(
        decode_body(
            br#"{"message":0,"finishReason":"stop"}"#,
            &mut response,
            &headers,
            &given
        ),
        Err(PegasusError::Bedrock { status: 200, .. })
    ));
}

fn content_errors() {
    let headers = header_counts("1", "2");
    let given = messages("[]");
    let mut without_choices = response(Vec::new());
    assert!(matches!(
        decode_body(
            br#"{"message":"visible"}"#,
            &mut without_choices,
            &headers,
            &given
        ),
        Err(PegasusError::Bedrock { status: 200, .. })
    ));
    let mut with_calls = single_choice();
    with_calls.choices[0].message.tool_calls = Some(Vec::new());
    assert!(matches!(
        decode_body(
            br#"{"message":"visible"}"#,
            &mut with_calls,
            &headers,
            &given
        ),
        Err(PegasusError::Bedrock { status: 200, .. })
    ));
    for body in [
        br#"{"message":""}"#.as_slice(),
        br#"{"finishReason":"stop"}"#.as_slice(),
    ] {
        let mut response = single_choice();
        let failure = decode_body(body, &mut response, &headers, &given).unwrap_err();
        assert!(matches!(failure, PegasusError::Bedrock { status: 200, .. }));
        assert_eq!(response.choices[0].message.content, None);
    }
}

fn content_catch_error() {
    let headers = header_counts("1", "2");
    let given = messages("[]");
    let failure = block_on(decode_pegasus_reply(
        br#"{"finishReason":"stop"}"#,
        &context(418, &headers, &given, 10.0),
        &mut single_choice(),
        &FixtureCounter::counting(0, 0),
    ))
    .unwrap_err();
    let PegasusError::Bedrock { status, error } = failure else {
        panic!("expected a content error, got {failure:?}");
    };
    assert_eq!(status, 418);
    assert_eq!(
        error.cause.headers,
        Some(ErrorHeaders::Http(headers.clone()))
    );
    assert!(
        error
            .cause
            .error
            .message
            .contains("Error setting response content")
    );
}

fn count_errors() {
    let given = messages("[]");
    assert!(matches!(
        decode_body(
            br#"{"message":"visible"}"#,
            &mut single_choice(),
            &header_counts("abc", "2"),
            &given
        ),
        Err(PegasusError::TokenCountHeader { .. })
    ));
    let failure = block_on(decode_pegasus_reply(
        br#"{"message":"visible"}"#,
        &context(200, &[], &given, 10.0),
        &mut single_choice(),
        &FixtureCounter::failing(),
    ));
    assert!(matches!(failure, Err(PegasusError::TokenCount { .. })));
}

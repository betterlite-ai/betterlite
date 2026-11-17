use betterlite_models::decisions::Json;
use http::HeaderMap;

use super::support::{
    BASE, Call, Consume, DATA, Inputs, Refusal, Reply, converse, header_map, invoke, messages, run,
};

const SECRET: &str = "fixture-credential-4e1d";
const TOKEN: &str = "fixture-session-token-9a2c";

fn request_headers() -> HeaderMap {
    let authorization = format!("Bearer {SECRET}");
    header_map(&[
        ("authorization", authorization.as_str()),
        ("x-amz-security-token", TOKEN),
    ])
}

fn echoed_headers() -> Vec<(String, String)> {
    vec![
        ("authorization".to_owned(), format!("Bearer {SECRET}")),
        ("x-amz-security-token".to_owned(), TOKEN.to_owned()),
    ]
}

#[test]
fn bedrock_http_stream_secret_safe_failure() {
    let messages = messages();
    let shown = vec![
        status_failure(&messages),
        transport_failure(&messages),
        empty_stream_failure(&messages),
        converse_failure(&messages),
        bad_reply_failure(&messages),
    ];
    let printed = shown.concat();
    assert!(
        !printed.contains(SECRET),
        "a failure path leaks the credential"
    );
    assert!(
        !printed.contains(TOKEN),
        "a failure path leaks the session token"
    );
}

fn status_failure(messages: &Json) -> String {
    // A status failure: the response headers echo the credential, and no failure path prints them.
    let mut reply = Reply::frames(&[]);
    reply.status = 403;
    reply.headers = echoed_headers();
    let inputs = Inputs::new(reply);
    let mut call = invoke(BASE, DATA, messages);
    call.headers = request_headers();
    let outcome = run(Call::Invoke(call), &inputs, Consume::None);
    let error = outcome.error.as_ref().unwrap();
    assert_eq!(outcome.provider_status(), Some(403));
    format!("{error:?}{error}")
}

fn transport_failure(messages: &Json) -> String {
    // A transport failure before any response.
    let transport_inputs = Inputs::new(Reply::refusing(Refusal::Closed));
    let mut transport_call = invoke(BASE, DATA, messages);
    transport_call.headers = request_headers();
    let transport_outcome = run(
        Call::Invoke(transport_call),
        &transport_inputs,
        Consume::None,
    );
    let error = transport_outcome.error.unwrap();
    format!("{error:?}")
}

fn empty_stream_failure(messages: &Json) -> String {
    // A stream that decoded no events, and the log entry written when it opened.
    let mut empty_reply = Reply::frames(&[]);
    empty_reply.headers = echoed_headers();
    let empty_inputs = Inputs::new(empty_reply);
    let mut empty_call = invoke(BASE, DATA, messages);
    empty_call.headers = request_headers();
    let empty_outcome = run(Call::Invoke(empty_call), &empty_inputs, Consume::All);
    assert_eq!(empty_outcome.failure_statuses(), vec![Some(502)]);
    let failures = &empty_outcome.failures;
    let first = &failures[0];
    let logs = serde_json::to_string(&empty_outcome.logs).unwrap();
    format!("{failures:?}{first:?}{logs}")
}

fn converse_failure(messages: &Json) -> String {
    // Converse hands its failures through unwrapped.
    let mut converse_reply = Reply::buffered("denied");
    converse_reply.status = 400;
    converse_reply.headers = echoed_headers();
    let converse_inputs = Inputs::new(converse_reply);
    let mut converse_call = converse(messages);
    converse_call.headers = request_headers();
    let converse_outcome = run(
        Call::Converse(converse_call),
        &converse_inputs,
        Consume::None,
    );
    assert_eq!(converse_outcome.provider_message(), Some("b'denied'"));
    let error = converse_outcome.error.unwrap();
    format!("{error:?}")
}

fn bad_reply_failure(messages: &Json) -> String {
    // A reply the converter rejects carries the converter's own text, not the request.
    let bad_inputs = Inputs::new(Reply::buffered("[]"));
    let mut bad_call = converse(messages);
    bad_call.fake_stream = true;
    bad_call.headers = request_headers();
    let bad_outcome = run(Call::Converse(bad_call), &bad_inputs, Consume::None);
    assert_eq!(bad_outcome.provider_status(), Some(422));
    let error = bad_outcome.error.unwrap();
    format!("{error:?}")
}

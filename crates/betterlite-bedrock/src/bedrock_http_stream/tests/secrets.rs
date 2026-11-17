use http::HeaderMap;
use serde_json::to_value;

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
    let mut shown = Vec::new();

    // A status failure: the response headers echo the credential, and no failure path prints them.
    let mut reply = Reply::streaming(&[]);
    reply.status = 403;
    reply.headers = echoed_headers();
    let inputs = Inputs::new(reply);
    let mut call = invoke(BASE, DATA, &messages);
    call.headers = request_headers();
    let outcome = run(Call::Invoke(call), &inputs, Consume::None);
    let error = outcome.error.as_ref().unwrap();
    assert_eq!(outcome.provider_status(), Some(403));
    shown.push(format!("{error:?}"));
    shown.push(format!("{error}"));

    // A transport failure before any response.
    let inputs = Inputs::new(Reply::refusing(Refusal::Closed));
    let mut call = invoke(BASE, DATA, &messages);
    call.headers = request_headers();
    let outcome = run(Call::Invoke(call), &inputs, Consume::None);
    shown.push(format!("{:?}", outcome.error.unwrap()));

    // A stream that decoded no events, and the log entry written when it opened.
    let mut reply = Reply::streaming(&[]);
    reply.headers = echoed_headers();
    let inputs = Inputs::new(reply);
    let mut call = invoke(BASE, DATA, &messages);
    call.headers = request_headers();
    let outcome = run(Call::Invoke(call), &inputs, Consume::All);
    assert_eq!(outcome.failure_statuses(), vec![Some(502)]);
    shown.push(format!("{:?}", outcome.failures));
    shown.push(format!("{:?}", outcome.failures[0]));
    shown.push(to_value(&outcome.logs).unwrap().to_string());

    // Converse hands its failures through unwrapped.
    let mut reply = Reply::buffered("denied");
    reply.status = 400;
    reply.headers = echoed_headers();
    let inputs = Inputs::new(reply);
    let mut call = converse(BASE, DATA, &messages);
    call.headers = request_headers();
    let outcome = run(Call::Converse(call), &inputs, Consume::None);
    assert_eq!(outcome.provider_message(), Some("b'denied'"));
    shown.push(format!("{:?}", outcome.error.unwrap()));

    // A reply the converter rejects carries the converter's own text, not the request.
    let inputs = Inputs::new(Reply::buffered("[]"));
    let mut call = converse(BASE, DATA, &messages);
    call.fake_stream = true;
    call.headers = request_headers();
    let outcome = run(Call::Converse(call), &inputs, Consume::None);
    assert_eq!(outcome.provider_status(), Some(422));
    shown.push(format!("{:?}", outcome.error.unwrap()));

    for text in &shown {
        assert!(
            !text.contains(SECRET),
            "a failure path leaks the credential: {text}"
        );
        assert!(
            !text.contains(TOKEN),
            "a failure path leaks the session token: {text}"
        );
    }
}

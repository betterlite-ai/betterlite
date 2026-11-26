#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::decisions::Json;
use http::header::HeaderValue;

use crate::api::decisions_send::format::DecisionsOutput;
use crate::decisions::DecisionsArgs;
use crate::decisions::decisions_entrypoint::test_support::{
    API_BASE, Fixture, Mode, OPENAI_ANSWER, body, decisions_args, text,
};

fn full_args() -> DecisionsArgs {
    let mut args = decisions_args("openai");
    args.safety_identifier = Some(String::from("user-7"));
    args.extra_headers = Some(BTreeMap::from([(
        String::from("x-test"),
        String::from("1"),
    )]));
    args.kwargs
        .insert(String::from("drop_params"), Json::Bool(true));
    args
}

#[test]
fn decisions_takes_every_documented_parameter() {
    let mut fixture = Fixture::new();
    fixture.clients.answer(200, OPENAI_ANSWER);
    let output = fixture.decisions(full_args()).expect("the call succeeds");
    let DecisionsOutput::OpenAi(response) = output else {
        panic!("an OpenAI-format request answers in the OpenAI format");
    };
    assert_eq!(response.model, "model-1");
    let sent = fixture.clients.call();
    assert_eq!(sent[0].mode, Mode::Sync);
    assert_eq!(sent[0].request.timeout, Some(Duration::from_secs(5)));
    assert_eq!(
        sent[0].request.url.as_str(),
        format!("{API_BASE}/v1/decisions")
    );
    assert_eq!(
        sent[0].request.headers.get("x-test"),
        Some(&HeaderValue::from_static("1"))
    );
    let Json::Object(fields) = body(&sent[0].request) else {
        panic!("the sent body is an object");
    };
    drop(sent);
    assert_eq!(fields.get("safety_identifier"), Some(&text("user-7")));
}

#[tokio::test]
async fn adecisions_takes_every_documented_parameter() {
    let mut fixture = Fixture::new();
    fixture.clients.answer(200, OPENAI_ANSWER);
    let output = fixture
        .adecisions(full_args())
        .await
        .expect("the call succeeds");
    let DecisionsOutput::OpenAi(response) = output else {
        panic!("an OpenAI-format request answers in the OpenAI format");
    };
    assert_eq!(response.model, "model-1");
    assert_eq!(fixture.clients.call()[0].mode, Mode::Async);
}

#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::api::decisions_send::format::DecisionsOutput;
use crate::decisions::decisions_entrypoint::test_support::{
    API_BASE, Fixture, Mode, body, decisions_args, systemone_questions, text,
};

#[test]
fn decisions_sends_the_openai_request_through_the_blocking_client() {
    let mut fixture = Fixture::new();
    let output = fixture
        .decisions(decisions_args("typesafe"))
        .expect("the call succeeds");
    let DecisionsOutput::OpenAi(response) = output else {
        panic!("an OpenAI-format request answers in the OpenAI format");
    };
    assert_eq!(response.model, "model-1");
    let sent = fixture.clients.call();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].mode, Mode::Sync);
    assert_eq!(
        sent[0].request.url.as_str(),
        format!("{API_BASE}/v1/systemone")
    );
    let Json::Object(fields) = body(&sent[0].request) else {
        panic!("the sent body is an object");
    };
    assert_eq!(fields.get("input"), None);
    assert_eq!(fields.get("state"), Some(&text("state")));
}

#[test]
fn decisions_with_a_state_keyword_returns_the_systemone_response() {
    let mut fixture = Fixture::new();
    let mut args = decisions_args("typesafe");
    args.input = None;
    args.questions = Some(systemone_questions());
    args.kwargs.insert(String::from("state"), text("state"));
    let output = fixture.decisions(args).expect("the call succeeds");
    let DecisionsOutput::SystemOne(response) = output else {
        panic!("a state keyword answers in the System One format");
    };
    let names: Vec<String> = response.answers.keys().cloned().collect();
    assert_eq!(names, vec![String::from("first")]);
    assert_eq!(fixture.clients.call()[0].mode, Mode::Sync);
}

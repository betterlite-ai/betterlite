#![cfg(test)]

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;

use crate::api::decisions_send::format::DecisionsOutput;
use crate::decisions::decisions_entrypoint::test_support::{
    API_BASE, Fixture, Mode, REJECTION, body, decisions_args, systemone_questions, text,
};

#[tokio::test]
async fn adecisions_sends_the_openai_request_and_returns_the_openai_response() {
    let mut fixture = Fixture::new();
    let output = fixture
        .adecisions(decisions_args("typesafe"))
        .await
        .expect("the call succeeds");
    let DecisionsOutput::OpenAi(response) = output else {
        panic!("an OpenAI-format request answers in the OpenAI format");
    };
    assert_eq!(response.model, "model-1");
    assert_eq!(response.answers.len(), 1);
    assert_eq!(
        response.hidden_params.get("model"),
        Some(&text("typesafe/model-1"))
    );
    let sent = fixture.clients.call();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].mode, Mode::Async);
    assert_eq!(
        sent[0].request.url.as_str(),
        format!("{API_BASE}/v1/systemone")
    );
    let Json::Object(fields) = body(&sent[0].request) else {
        panic!("the sent body is an object");
    };
    assert_eq!(fields.get("model"), Some(&text("model-1")));
    assert_eq!(fields.get("state"), Some(&text("state")));
    assert_eq!(fields.get("input"), None);
}

#[tokio::test]
async fn adecisions_with_a_state_keyword_returns_the_systemone_response() {
    let mut fixture = Fixture::new();
    let mut args = decisions_args("typesafe");
    args.input = None;
    args.questions = Some(systemone_questions());
    args.kwargs.insert(String::from("state"), text("state"));
    let output = fixture.adecisions(args).await.expect("the call succeeds");
    let DecisionsOutput::SystemOne(response) = output else {
        panic!("a state keyword answers in the System One format");
    };
    let names: Vec<String> = response.answers.keys().cloned().collect();
    assert_eq!(names, vec![String::from("first")]);
    assert_eq!(
        response.hidden_params.get("custom_llm_provider"),
        Some(&text("typesafe"))
    );
}

#[tokio::test]
async fn adecisions_maps_a_rejected_call_to_its_error_class() {
    let mut fixture = Fixture::new();
    fixture.clients.answer(401, REJECTION);
    let error = fixture
        .adecisions(decisions_args("typesafe"))
        .await
        .expect_err("the provider rejects the call");
    assert!(matches!(error.class(), ErrorClass::Authentication(_)));
    assert_eq!(error.class().metadata().status_code, 401);
    assert!(
        error
            .to_string()
            .starts_with("litellm.AuthenticationError: "),
        "{error}"
    );
    assert_eq!(fixture.clients.call().len(), 1);
}

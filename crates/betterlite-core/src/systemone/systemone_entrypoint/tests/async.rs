#![cfg(test)]

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;

use crate::decisions::decisions_entrypoint::test_support::{
    API_BASE, Fixture, Mode, REJECTION, body, systemone_args, text,
};

#[tokio::test]
async fn asystemone_sends_the_systemone_request_and_returns_the_answer() {
    let mut fixture = Fixture::new();
    let response = fixture
        .asystemone(systemone_args("typesafe"))
        .await
        .expect("the call succeeds");
    let names: Vec<String> = response.answers.keys().cloned().collect();
    assert_eq!(names, vec![String::from("first")]);
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
    let Some(Json::Object(questions)) = fields.get("questions") else {
        panic!("the sent body carries the question map");
    };
    assert!(questions.contains_key("first"));
}

#[tokio::test]
async fn asystemone_maps_a_rejected_call_to_its_error_class() {
    let mut fixture = Fixture::new();
    fixture.clients.answer(401, REJECTION);
    let error = fixture
        .asystemone(systemone_args("typesafe"))
        .await
        .expect_err("the provider rejects the call");
    assert!(matches!(error.class(), ErrorClass::Authentication(_)));
    assert_eq!(fixture.clients.call().len(), 1);
}

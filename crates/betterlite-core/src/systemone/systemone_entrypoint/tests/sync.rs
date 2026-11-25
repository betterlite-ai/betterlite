#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::decisions::decisions_entrypoint::test_support::{
    API_BASE, Fixture, Mode, body, systemone_args, text,
};

#[test]
fn systemone_sends_through_the_blocking_client() {
    let mut fixture = Fixture::new();
    let response = fixture
        .systemone(systemone_args("typesafe"))
        .expect("the call succeeds");
    let names: Vec<String> = response.answers.keys().cloned().collect();
    assert_eq!(names, vec![String::from("first")]);
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
    assert_eq!(fields.get("state"), Some(&text("state")));
}

#[test]
fn systemone_without_a_provider_uses_the_typesafe_defaults() {
    let mut fixture = Fixture::new();
    let mut args = systemone_args("typesafe");
    args.api_base = None;
    let response = fixture
        .systemone(args)
        .expect("the default api base applies");
    assert_eq!(response.model.as_deref(), Some("model-1"));
    assert_eq!(
        fixture.clients.call()[0].request.url.as_str(),
        "https://api.typesafe.ai/v1/systemone"
    );
}

use betterlite_models::decisions::Json;
use betterlite_openai::error::OpenAiError;
use betterlite_openai::response_resources::error::ResponseResourceError;
use betterlite_openai::response_resources::request::ListInputItemsQuery;

use super::run;
use super::support::{
    Events, Fixture, Reply, Source, Wire, capture, input_items_call, json, openai_config, params,
    retrieve_call,
};
use crate::api::responses_read_http::config::ReadEnvironment;
use crate::api::responses_read_http::error::ReadError;
use crate::api::responses_read_http::service::{ClientKind, ReadCall, ReadDeps, ReadOutcome};

pub(super) async fn failure_boundary(mode: ClientKind) {
    status_error(mode).await;
    malformed(mode).await;
    invalid_record(mode).await;
    input_items_object(mode).await;
}

async fn status_error(mode: ClientKind) {
    let (retrieve_capture, _retrieve_guard) = capture();
    let reply = Reply::json(429, "slow down")
        .with_header("retry-after", "3")
        .with_header("content-type", "text/plain");
    let api = params("http://mock.test/v1", "sk-provider");
    let retrieve = retrieve_call(&api);
    let (wire, result) = read_with(mode, reply, &retrieve).await;
    let Err(ReadError::Provider(error)) = result else {
        panic!("a 429 is a provider error");
    };
    assert_eq!(error.status_code, 429);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "slow down");
    assert_eq!(
        error.headers.get("retry-after"),
        Some(&Json::Text(String::from("3")))
    );
    assert_eq!(wire.requests().len(), 1);
    assert_debug_line(mode, &retrieve_capture.messages());
    let (items_capture, _items_guard) = capture();
    let query = ListInputItemsQuery::default();
    let items = input_items_call(&api, &query);
    let (items_wire, items_result) = read_with(mode, Reply::json(429, "slow down"), &items).await;
    let Err(ReadError::Provider(items_error)) = items_result else {
        panic!("a 429 is a provider error for an input items read too");
    };
    assert_eq!(items_error.status_code, 429);
    assert_eq!(items_wire.requests().len(), 1);
    // The retrieve-only debug line never appears for a failed input items read
    assert!(
        !items_capture
            .messages()
            .iter()
            .any(|message| message.starts_with("Error retrieving response: "))
    );
}

fn assert_debug_line(mode: ClientKind, messages: &[String]) {
    let logged = messages
        .iter()
        .any(|message| message.starts_with("Error retrieving response: "));
    assert_eq!(logged, mode == ClientKind::Asynchronous);
    if logged {
        assert!(
            messages
                .iter()
                .any(|message| message == "Error retrieving response: slow down")
        );
    }
}

async fn malformed(mode: ClientKind) {
    let api = params("http://mock.test/v1", "sk-provider");
    let call = retrieve_call(&api);
    let (_, result) = read_with(mode, Reply::json(200, "{not json"), &call).await;
    let Err(ReadError::Response(ResponseResourceError::Provider(OpenAiError::Provider(provider)))) =
        result
    else {
        panic!("a body that is not JSON fails in the response transform");
    };
    assert_eq!(provider.status_code, 200);
    assert_eq!(provider.message, "{not json");
    assert!(provider.headers.is_none());
}

async fn invalid_record(mode: ClientKind) {
    let api = params("http://mock.test/v1", "sk-provider");
    let call = retrieve_call(&api);
    let (_, result) = read_with(
        mode,
        Reply::json(200, r#"{"created_at":9,"output":[]}"#),
        &call,
    )
    .await;
    assert!(matches!(
        result,
        Err(ReadError::Response(ResponseResourceError::Record { .. }))
    ));
}

async fn input_items_object(mode: ClientKind) {
    let api = params("http://mock.test/v1", "sk-provider");
    let query = ListInputItemsQuery::default();
    let call = input_items_call(&api, &query);
    let (_, result) = read_with(mode, Reply::json(200, r#"{"anything":true}"#), &call).await;
    let outcome = result.expect("an input items reply only needs a JSON object");
    let ReadOutcome::InputItems(value) = outcome else {
        panic!("an input items read returns the provider object");
    };
    assert_eq!(value, json(r#"{"anything":true}"#));
}

async fn read_with(
    mode: ClientKind,
    reply: Reply,
    call: &ReadCall<'_>,
) -> (Wire, Result<ReadOutcome, ReadError>) {
    let config = openai_config(ReadEnvironment::default());
    let events = Events::new();
    let wire = Wire::replying(reply, &events);
    let source = Source::new(wire.clone());
    let deps = ReadDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let mut fixture = Fixture::new("responses");
    let result = run(&mut fixture, &deps, mode, call, &events).await;
    (wire, result)
}

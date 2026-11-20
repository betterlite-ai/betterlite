use betterlite_models::decisions::Json;
use betterlite_openai::error::OpenAiError;
use betterlite_openai::response_resources::error::ResponseResourceError;
use betterlite_openai::response_resources::request::ListInputItemsQuery;

use super::support::{
    Events, Fixture, Reply, Source, Wire, capture, json, openai_config, params, retrieve_call,
};
use super::{run, target};
use crate::api::responses_read_http::config::ReadEnvironment;
use crate::api::responses_read_http::error::ReadError;
use crate::api::responses_read_http::service::{
    ClientKind, InputItemsCall, ReadCall, ReadDeps, ReadOutcome,
};

pub(super) async fn failure_boundary(mode: ClientKind) {
    status_error(mode).await;
    input_items_status(mode).await;
    malformed(mode).await;
    invalid_record(mode).await;
    input_items_object(mode).await;
}

async fn status_error(mode: ClientKind) {
    let (captured, _guard) = capture();
    let reply = Reply::json(429, "slow down")
        .with_header("retry-after", "3")
        .with_header("content-type", "text/plain");
    let api = params("http://mock.test/v1", "sk-provider");
    let call = retrieve_call(&api);
    let (wire, result) = read_with(mode, reply, &call).await;
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
    assert_debug_line(mode, &captured.messages());
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

// The retrieve-only debug line never appears for a failed input items read
async fn input_items_status(mode: ClientKind) {
    let (captured, _guard) = capture();
    let api = params("http://mock.test/v1", "sk-provider");
    let query = ListInputItemsQuery::default();
    let call = ReadCall::InputItems(InputItemsCall {
        target: target("resp_a", &api, None, None),
        query: &query,
    });
    let (wire, result) = read_with(mode, Reply::json(429, "slow down"), &call).await;
    let Err(ReadError::Provider(error)) = result else {
        panic!("a 429 is a provider error for an input items read too");
    };
    assert_eq!(error.status_code, 429);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(wire.requests().len(), 1);
    assert!(
        !captured
            .messages()
            .iter()
            .any(|message| message.starts_with("Error retrieving response: "))
    );
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
    let call = ReadCall::InputItems(InputItemsCall {
        target: target("resp_a", &api, None, None),
        query: &query,
    });
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

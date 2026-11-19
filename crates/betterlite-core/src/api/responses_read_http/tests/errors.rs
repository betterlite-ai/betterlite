use betterlite_models::decisions::Json;
use betterlite_openai::error::OpenAiError;
use betterlite_openai::response_resources::error::ResponseResourceError;
use betterlite_openai::response_resources::request::ListInputItemsQuery;

use super::support::{Events, Fixture, Reply, Source, Wire, capture, json, openai_config, params};
use super::{run, target};
use crate::api::responses_read_http::config::ReadEnvironment;
use crate::api::responses_read_http::error::ReadError;
use crate::api::responses_read_http::service::{
    ClientKind, InputItemsCall, ReadCall, ReadDeps, ReadOutcome, RetrieveCall,
};

pub(super) async fn failure_boundary(mode: ClientKind) {
    status_error(mode).await;
    malformed(mode).await;
    invalid_record(mode).await;
    input_items_object(mode).await;
}

async fn status_error(mode: ClientKind) {
    let (captured, _guard) = capture();
    let reply = Reply::json(429, "slow down")
        .with_header("retry-after", "3")
        .with_header("content-type", "text/plain");
    let (wire, result) = retrieve_with(mode, reply).await;
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

async fn malformed(mode: ClientKind) {
    let (_, result) = retrieve_with(mode, Reply::json(200, "{not json")).await;
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
    let (_, result) =
        retrieve_with(mode, Reply::json(200, r#"{"created_at":9,"output":[]}"#)).await;
    assert!(matches!(
        result,
        Err(ReadError::Response(ResponseResourceError::Record { .. }))
    ));
}

async fn input_items_object(mode: ClientKind) {
    let config = openai_config(ReadEnvironment::default());
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, r#"{"anything":true}"#), &events);
    let source = Source::new(wire);
    let deps = ReadDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let api = params("http://mock.test/v1", "sk-provider");
    let mut fixture = Fixture::new("responses");
    let query = ListInputItemsQuery::default();
    let call = ReadCall::InputItems(InputItemsCall {
        target: target("resp_a", &api, None, None),
        query: &query,
    });
    let outcome = run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("an input items reply only needs a JSON object");
    let ReadOutcome::InputItems(value) = outcome else {
        panic!("an input items read returns the provider object");
    };
    assert_eq!(value, json(r#"{"anything":true}"#));
}

async fn retrieve_with(mode: ClientKind, reply: Reply) -> (Wire, Result<ReadOutcome, ReadError>) {
    let config = openai_config(ReadEnvironment::default());
    let events = Events::new();
    let wire = Wire::replying(reply, &events);
    let source = Source::new(wire.clone());
    let deps = ReadDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let api = params("http://mock.test/v1", "sk-provider");
    let mut fixture = Fixture::new("responses");
    let call = ReadCall::Retrieve(RetrieveCall {
        target: target("resp_a", &api, None, None),
        extra_body: None,
    });
    let result = run(&mut fixture, &deps, mode, &call, &events).await;
    (wire, result)
}

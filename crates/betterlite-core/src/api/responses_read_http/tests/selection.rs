use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use betterlite_openai::response_resources::request::ListInputItemsQuery;

use super::support::{
    Events, Fixture, IMPATIENT, ProxiedSource, Reply, Source, Wire, capture, openai_config, params,
};
use super::{run, target};
use crate::api::responses_read_http::config::{ReadEnvironment, ReadParams};
use crate::api::responses_read_http::error::ReadError;
use crate::api::responses_read_http::service::{
    ClientKind, InputItemsCall, ReadCall, ReadDeps, RetrieveCall, SuppliedClient,
};

const RETRIEVED: &str =
    r#"{"id":"resp_a","created_at":9,"status":"completed","model":"gpt-4o","output":[]}"#;
const BASE: &str = "http://mock.test/v1/responses";

pub(super) async fn client_selection(mode: ClientKind) {
    supplied_wins(mode).await;
    pool_wins(mode, None).await;
    pool_wins(mode, Some(false)).await;
    wrong_kind_uses_pool(mode).await;
    debug_lines(mode).await;
    no_per_call_timeout(mode).await;
}

async fn supplied_wins(mode: ClientKind) {
    let config = openai_config(ReadEnvironment::default());
    let events = Events::new();
    let pooled = Wire::replying(Reply::json(200, RETRIEVED), &events);
    let supplied = Wire::replying(Reply::json(200, RETRIEVED), &events);
    let source = Source::new(pooled.clone());
    let deps = ReadDeps {
        config: &config,
        source: &source,
        supplied: Some(SuppliedClient {
            kind: mode,
            transport: &supplied,
        }),
    };
    let api = params("http://mock.test/v1", "sk-provider");
    let mut fixture = Fixture::new("responses");
    let call = retrieve_call(&api);
    run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the supplied client sends the retrieve");
    assert_eq!(supplied.requests().len(), 1);
    assert!(pooled.requests().is_empty());
    assert!(source.calls().is_empty());
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text(String::from(BASE)))
    );
}

async fn pool_wins(mode: ClientKind, ssl_verify: Option<bool>) {
    let config = openai_config(ReadEnvironment::default());
    let events = Events::new();
    let pooled = Wire::replying(Reply::json(200, RETRIEVED), &events);
    let source = Source::new(pooled.clone());
    let deps = ReadDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let mut api = params("http://mock.test/v1", "sk-provider");
    api.ssl_verify = ssl_verify;
    let mut fixture = Fixture::new("responses");
    let call = retrieve_call(&api);
    run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the pool sends the retrieve");
    assert_eq!(source.calls(), vec![ssl_verify]);
    assert_eq!(pooled.requests().len(), 1);
}

async fn wrong_kind_uses_pool(mode: ClientKind) {
    let config = openai_config(ReadEnvironment::default());
    let events = Events::new();
    let pooled = Wire::replying(Reply::json(200, RETRIEVED), &events);
    let supplied = Wire::replying(Reply::json(200, RETRIEVED), &events);
    let source = Source::new(pooled.clone());
    let wrong = match mode {
        ClientKind::Sync => ClientKind::Asynchronous,
        ClientKind::Asynchronous => ClientKind::Sync,
    };
    let deps = ReadDeps {
        config: &config,
        source: &source,
        supplied: Some(SuppliedClient {
            kind: wrong,
            transport: &supplied,
        }),
    };
    let api = params("http://mock.test/v1", "sk-provider");
    let mut fixture = Fixture::new("responses");
    let call = retrieve_call(&api);
    run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("a client of the other kind never sends this read");
    assert_eq!(source.calls(), vec![None]);
    assert_eq!(pooled.requests().len(), 1);
    assert!(supplied.requests().is_empty());
}

async fn debug_lines(mode: ClientKind) {
    let (captured, _guard) = capture();
    let config = openai_config(ReadEnvironment::default());
    let events = Events::new();
    let pooled = Wire::replying(Reply::json(200, RETRIEVED), &events);
    let source = Source::new(pooled);
    let deps = ReadDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let api = params("http://mock.test/v1", "sk-provider");
    let mut fixture = Fixture::new("responses");
    let retrieve = retrieve_call(&api);
    run(&mut fixture, &deps, mode, &retrieve, &events)
        .await
        .expect("the retrieve succeeds");
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text(String::from(BASE)))
    );
    let query = ListInputItemsQuery::default();
    let items = ReadCall::InputItems(InputItemsCall {
        target: target("resp_a", &api, None, None),
        query: &query,
    });
    run(&mut fixture, &deps, mode, &items, &events)
        .await
        .expect("the input items read succeeds");
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text(String::from(BASE)))
    );
    let messages = captured.messages();
    let expected = mode == ClientKind::Asynchronous;
    for label in ["get_responses", "list_input_items"] {
        let line = format!("Creating HTTP client for {label} with shared_session: None");
        assert_eq!(messages.iter().any(|message| message == &line), expected);
    }
}

async fn no_per_call_timeout(mode: ClientKind) {
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], RETRIEVED),
        Duration::from_millis(300),
    )
    .expect("the fixture serves");
    let config = openai_config(ReadEnvironment::default());
    let source = ProxiedSource::new(IMPATIENT);
    let deps = ReadDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let api = params(&format!("http://{}/v1", serving.address), "sk-provider");
    let mut fixture = Fixture::new("responses");
    let events = Events::new();
    let call = ReadCall::Retrieve(RetrieveCall {
        target: target("resp_a", &api, None, Some(Duration::from_secs(7))),
        extra_body: None,
    });
    let result = run(&mut fixture, &deps, mode, &call, &events).await;
    let Err(ReadError::Provider(error)) = result else {
        panic!("the client's own read timeout ends the GET");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(
        error.message,
        "the request ran past its read timeout or total deadline"
    );
}

fn retrieve_call<'call>(api: &'call ReadParams) -> ReadCall<'call> {
    ReadCall::Retrieve(RetrieveCall {
        target: target("resp_a", api, None, None),
        extra_body: None,
    })
}

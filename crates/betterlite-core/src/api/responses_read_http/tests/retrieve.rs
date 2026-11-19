use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::decisions::Json;
use http::Method;

use super::support::{
    Events, Fixture, Reply, Source, Wire, caller_headers, extra_body, header, json, openai_config,
    params,
};
use super::{run, target};
use crate::api::responses_read_http::config::ReadEnvironment;
use crate::api::responses_read_http::service::{
    ClientKind, ReadCall, ReadDeps, ReadOutcome, RetrieveCall,
};

const RETRIEVED: &str =
    r#"{"id":"resp_a","created_at":9,"status":"completed","model":"gpt-4o","output":[]}"#;

pub(super) async fn headers_and_response(mode: ClientKind) {
    let config = openai_config(ReadEnvironment::default());
    let events = Events::new();
    let wire = Wire::replying(
        Reply::json(200, RETRIEVED).with_header("x-request-id", "req_a"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = ReadDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let api = params("http://mock.test/v1", "sk-provider");
    let caller = caller_headers(&[
        ("Authorization", "Bearer sk-caller"),
        ("X-Trace", "trace-a"),
    ]);
    let body = extra_body(r#"{"ignored":true}"#);
    let mut fixture = Fixture::new("responses");
    let call = ReadCall::Retrieve(RetrieveCall {
        target: target("resp/a", &api, Some(&caller), Some(Duration::from_secs(7))),
        extra_body: Some(&body),
    });
    let outcome = run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the retrieve succeeds");
    assert_reply(&outcome);
    assert_request(&wire);
    assert_logging(&fixture);
}

fn assert_reply(outcome: &ReadOutcome) {
    let ReadOutcome::Response(reply) = outcome else {
        panic!("a retrieve returns the typed reply");
    };
    assert_eq!(reply.response.id, "resp_a");
    assert_eq!(reply.response.created_at, 9_i64);
    assert_eq!(reply.response.status.as_deref(), Some("completed"));
    assert_eq!(reply.response.model.as_deref(), Some("gpt-4o"));
    assert_eq!(
        reply.headers.get("x-request-id").map(String::as_str),
        Some("req_a")
    );
    assert_eq!(
        reply.additional_headers.get("llm_provider-x-request-id"),
        Some(&Json::Text("req_a".to_owned()))
    );
}

fn assert_request(wire: &Wire) {
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    let sent = &requests[0];
    assert_eq!(sent.method, Method::GET);
    assert_eq!(sent.url.as_str(), "http://mock.test/v1/responses/resp%2Fa");
    assert!(sent.query.is_empty());
    assert!(matches!(sent.body, RequestBody::Empty));
    assert_eq!(
        header(&sent.headers, "authorization"),
        Some("Bearer sk-provider")
    );
    assert_eq!(
        header(&sent.headers, "content-type"),
        Some("application/json")
    );
    assert_eq!(header(&sent.headers, "x-trace"), Some("trace-a"));
}

fn assert_logging(fixture: &Fixture) {
    assert_eq!(
        fixture.logged("api_base"),
        Some(&json(r#""http://mock.test/v1/responses""#))
    );
    assert_eq!(fixture.input(), Some(&json(r#""""#)));
    assert_eq!(fixture.logged("complete_input_dict"), Some(&json("{}")));
    assert_eq!(
        fixture.logged("headers"),
        Some(&json(
            r#"{"Authorization":"Be****er","Content-Type":"application/json","X-Trace":"trace-a"}"#
        ))
    );
}

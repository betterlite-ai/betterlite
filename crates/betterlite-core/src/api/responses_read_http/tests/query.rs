use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_openai::response_resources::request::{InputItemsOrder, ListInputItemsQuery};
use http::Method;

use super::support::{Events, Fixture, Reply, Source, Wire, json, openai_config, params};
use super::{run, target};
use crate::api::responses_read_http::config::ReadEnvironment;
use crate::api::responses_read_http::service::{
    ClientKind, InputItemsCall, ReadCall, ReadDeps, ReadOutcome,
};

const LIST: &str = r#"{"object":"list","data":[],"has_more":false}"#;

pub(super) async fn input_items(mode: ClientKind) {
    let config = openai_config(ReadEnvironment::default());
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, LIST), &events);
    let source = Source::new(wire.clone());
    let deps = ReadDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let mut fixture = Fixture::new("responses");
    let api = params("http://mock.test/v1", "sk-provider");
    let cases = input_item_cases();
    let reads = cases.len();
    for (query, logged) in cases {
        let call = ReadCall::InputItems(InputItemsCall {
            target: target("resp_a", &api, None, Some(Duration::from_secs(7))),
            query: &query,
        });
        let outcome = run(&mut fixture, &deps, mode, &call, &events)
            .await
            .expect("the input items read succeeds");
        let ReadOutcome::InputItems(value) = outcome else {
            panic!("an input items read returns the provider object");
        };
        assert_eq!(value, json(LIST));
        assert_eq!(fixture.logged("complete_input_dict"), Some(&json(logged)));
    }
    assert_requests(&wire);
    let mut expected = Vec::new();
    for _ in 0..reads {
        expected.push(String::from("log"));
        expected.push(String::from("send"));
    }
    assert_eq!(events.recorded(), expected);
}

fn input_item_cases() -> [(ListInputItemsQuery, &'static str); 3] {
    [
        (
            ListInputItemsQuery::default(),
            r#"{"limit":20,"order":"desc"}"#,
        ),
        (
            ListInputItemsQuery {
                after: Some(String::new()),
                before: Some(String::from("b")),
                include: Some(vec![
                    String::from("message.input_image.image_url"),
                    String::from("computer_call_output.output.image_url"),
                ]),
                limit: Some(3),
                order: Some(InputItemsOrder::Asc),
            },
            r#"{"after":"","before":"b","include":"message.input_image.image_url,computer_call_output.output.image_url","limit":3,"order":"asc"}"#,
        ),
        (
            ListInputItemsQuery {
                after: None,
                before: None,
                include: Some(Vec::new()),
                limit: None,
                order: None,
            },
            "{}",
        ),
    ]
}

// Every read sends one GET with a body-less request, and the timeout never becomes a wire argument
fn assert_requests(wire: &Wire) {
    let requests = wire.requests();
    assert_eq!(requests.len(), 3);
    for sent in requests.as_slice() {
        assert_eq!(sent.method, Method::GET);
        assert_eq!(sent.url.path(), "/v1/responses/resp_a/input_items");
        assert!(matches!(sent.body, RequestBody::Empty));
    }
    assert_eq!(
        requests[0].query,
        vec![
            (String::from("limit"), String::from("20")),
            (String::from("order"), String::from("desc")),
        ]
    );
    assert_eq!(
        requests[1].query,
        vec![
            (String::from("after"), String::new()),
            (String::from("before"), String::from("b")),
            (
                String::from("include"),
                String::from("message.input_image.image_url,computer_call_output.output.image_url"),
            ),
            (String::from("limit"), String::from("3")),
            (String::from("order"), String::from("asc")),
        ]
    );
    assert!(requests[2].query.is_empty());
    drop(requests);
}

#![cfg(test)]

use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_openai::search::records::SearchResponse;

use super::support::{ANSWER, Harness, Recorder, Registry, object, request};

#[tokio::test]
async fn sync_return_is_the_handler_response_unchanged() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let hidden = object(r#"{"headers":{"x-request-id":"abc"}}"#);
    let registry = Registry::new(Recorder::new(false, hidden.clone()));
    let mut harness = Harness::new();
    let response = harness
        .search(
            &request(r#""rust""#, &format!("http://{}", served.address)),
            &registry,
        )
        .await
        .unwrap();
    let mut expected: SearchResponse = serde_json::from_str(ANSWER).unwrap();
    expected.hidden_params = hidden;
    assert_eq!(response, expected);
    assert_eq!(response.hidden_params, expected.hidden_params);
    assert_eq!(response.extra, object(r#"{"took":3}"#));
    assert_eq!(response.object.0, "search");
    served.finish().expect("the fixture answers");
}

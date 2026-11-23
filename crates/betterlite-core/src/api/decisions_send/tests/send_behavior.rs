use std::sync::Mutex;
use std::time::Duration;

use betterlite_http::interactions::base::{MAX_RESPONSE_BYTES, RequestFailure};
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::controls::json_value::JsonValue;
use http::header::{HeaderMap, HeaderValue};
use secrecy::SecretString;
use serde_json::Number;

use super::support::*;
use crate::api::decisions_send::clients::finish_blocking;
use crate::api::decisions_send::format::DecisionsOutput;
use crate::api::decisions_send::send::DecisionsSender;

// send_behavior: one POST of the transformed body with the validated headers

#[test]
fn send_behavior_posts_transformed_body_with_bearer_headers() {
    let serving = serve(reply("200 OK", &[], SYSTEMONE_ANSWER), Duration::default())
        .expect("the mock server starts");
    let api_base = format!("http://{}", serving.address);
    let environment = Environment::new();
    let key = SecretString::from("test-key".to_owned());
    let logging = Mutex::new(logging_state());
    let decisions = typesafe_call(&api_base, &key, &logging, &environment);
    let expected_body = serde_json::to_string(&decisions.body).expect("the wire serializes");
    let mut clients = real_clients();
    let log = LogEnv::for_events(&Events::new());
    let env = log.env();
    let mut sender = DecisionsSender::new(&mut clients, &env);
    let output = sender.send(&decisions).expect("the mock answers");
    match output {
        DecisionsOutput::SystemOne(response) => {
            assert_eq!(answer_names(&response), vec![String::from("first")]);
            let usage = response.usage.expect("the conversion sets usage");
            assert_eq!(usage.input_tokens.0, Number::from(3_u8));
        }
        DecisionsOutput::OpenAi(_) => {
            panic!("a SystemOne request answers with a SystemOne response")
        }
    }
    let request = serving.finish().expect("the server saw the request");
    assert_eq!(request.line, "POST /v1/systemone HTTP/1.1");
    let head = request.head.to_lowercase();
    assert!(head.contains("authorization: bearer test-key"));
    assert!(head.contains("content-type: application/json"));
    assert_eq!(request.body, expected_body);
}

#[tokio::test]
async fn send_behavior_async_answers_and_selects_the_provider_client() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    let output = fixture.asend(&decisions).await.expect("the call answers");
    assert!(matches!(output, DecisionsOutput::SystemOne(_)));
    let selections = fixture.clients.selections();
    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].0, Mode::Asynchronous);
    assert_eq!(
        selections[0].1.get("custom_llm_provider"),
        Some(&JsonValue::Text(String::from("typesafe")))
    );
}

#[test]
fn send_behavior_sync_selects_the_default_client() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let selections = fixture.clients.selections();
    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].0, Mode::Sync);
    assert!(selections[0].1.is_empty());
}

#[test]
fn send_behavior_passes_the_timeout_and_posts_once() {
    let mut fixture = Fixture::new();
    let mut decisions = systemone_call("typesafe", None);
    decisions.timeout = Some(Duration::from_secs(2));
    fixture.send(&decisions).expect("the call answers");
    let requests = fixture.wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout, Some(Duration::from_secs(2)));
    let sent = serde_json::to_string(&decisions.body).expect("the wire serializes");
    assert_eq!(requests[0].body, sent.into_bytes());
}

#[test]
fn send_behavior_blocking_client_accepts_the_body_cap() {
    let at_cap = vec![b'a'; MAX_RESPONSE_BYTES];
    let answer =
        finish_blocking(at_cap.as_slice(), 200, HeaderMap::new()).expect("a body at the cap reads");
    assert_eq!(answer.body.len(), MAX_RESPONSE_BYTES);
    let over_cap = vec![b'a'; MAX_RESPONSE_BYTES + 1];
    assert!(matches!(
        finish_blocking(over_cap.as_slice(), 200, HeaderMap::new()),
        Err(RequestFailure::BodyTooLarge { limit }) if limit == MAX_RESPONSE_BYTES
    ));
}

#[test]
fn send_behavior_sends_the_signed_headers() {
    let mut fixture = Fixture::new();
    fixture.config.signature = Some(String::from("sig-1"));
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let requests = fixture.wire.requests();
    assert_eq!(
        requests[0].headers.get("x-signature"),
        Some(&HeaderValue::from_static("sig-1"))
    );
    assert_eq!(
        requests[0].headers.get("content-type"),
        Some(&HeaderValue::from_static("application/json"))
    );
}

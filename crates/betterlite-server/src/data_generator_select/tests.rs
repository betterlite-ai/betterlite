#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use axum::http::Request;
use axum::http::request::Parts;
use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::{AsyncDataGenerator, GeneratorInput, SelectDataGenerator};

struct Recorded {
    response: String,
    key: String,
    request_data: BTreeMap<String, JsonValue>,
    has_request: bool,
    responses_stream_errors: bool,
}

struct FakeGenerator {
    seen: Mutex<Option<Recorded>>,
}

impl FakeGenerator {
    fn new() -> Self {
        Self {
            seen: Mutex::new(None),
        }
    }

    fn seen(&self) -> Option<Recorded> {
        self.seen.lock().expect("record lock").take()
    }
}

impl AsyncDataGenerator<String> for FakeGenerator {
    type Response = String;
    type Stream = bool;

    fn generate(&self, input: GeneratorInput<String, String>) -> bool {
        let recorded = Recorded {
            response: input.response,
            key: input.key,
            request_data: input.request_data,
            has_request: input.request.is_some(),
            responses_stream_errors: input.responses_stream_errors,
        };
        *self.seen.lock().expect("record lock") = Some(recorded);
        input.responses_stream_errors
    }
}

fn request_parts() -> Parts {
    Request::builder()
        .uri("/v1/responses")
        .body(())
        .expect("request")
        .into_parts()
        .0
}

fn request_data() -> BTreeMap<String, JsonValue> {
    let mut data = BTreeMap::new();
    data.insert(
        String::from("model"),
        JsonValue::String(String::from("gpt-4o")),
    );
    data.insert(String::from("stream"), JsonValue::Bool(true));
    data
}

#[rstest]
#[case::responses_routes(true)]
#[case::every_other_route(false)]
fn select_calls_the_port_with_the_flag_it_was_built_with(#[case] flag: bool) {
    let selector = SelectDataGenerator {
        generator: FakeGenerator::new(),
        responses_stream_errors: flag,
    };
    let selected = selector.select(
        String::from("response"),
        String::from("key-1"),
        request_data(),
        Some(request_parts()),
    );
    assert_eq!(selected, flag);
    let seen = selector.generator.seen().expect("recorded input");
    assert_eq!(seen.response, "response");
    assert_eq!(seen.key, "key-1");
    assert!(seen.has_request);
    assert_eq!(seen.responses_stream_errors, flag);
    assert_eq!(
        seen.request_data.get("model"),
        Some(&JsonValue::String(String::from("gpt-4o")))
    );
    assert_eq!(
        seen.request_data.get("stream"),
        Some(&JsonValue::Bool(true))
    );
}

#[test]
fn select_forwards_a_missing_request() {
    let selector = SelectDataGenerator {
        generator: FakeGenerator::new(),
        responses_stream_errors: false,
    };
    let selected = selector.select(
        String::from("response"),
        String::from("key-2"),
        BTreeMap::new(),
        None,
    );
    assert!(!selected);
    let seen = selector.generator.seen().expect("recorded input");
    assert!(!seen.has_request);
    assert!(seen.request_data.is_empty());
}

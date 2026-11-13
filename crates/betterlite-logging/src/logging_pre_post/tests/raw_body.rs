use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::logging_pre_post::args::RawBody;
use crate::logging_pre_post::request::raw_request_body;

fn parsed(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is valid")
}

#[rstest]
#[case::missing(None, r#"{"error":"Received empty dictionary for raw request body"}"#)]
#[case::object_passes_through(Some(parsed_body(r#"{"model":"gpt-4o"}"#)), r#"{"model":"gpt-4o"}"#)]
#[case::text_is_parsed(Some(RawBody::Text(r#"{"model":"gpt-4o"}"#.to_owned())), r#"{"model":"gpt-4o"}"#)]
#[case::unparsable_text_reports_the_text(
    Some(RawBody::Text("not json".to_owned())),
    r#"{"error":"Unable to parse raw request body. Got - not json"}"#
)]
fn logging_pre_post_behavior_tests_behavior(#[case] data: Option<RawBody>, #[case] expected: &str) {
    assert_eq!(raw_request_body(data.as_ref()), parsed(expected));
}

fn parsed_body(text: &str) -> RawBody {
    serde_json::from_str(text).expect("the fixture is valid")
}

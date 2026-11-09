use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::logging_pre_post::args::RawBody;
use crate::logging_pre_post::request::{CurlRequest, curl_command};

const PREFIX: &str = "\n\nPOST Request Sent from LiteLLM:\ncurl -X POST \\\n";

#[rstest]
#[case::masked_headers_and_text_body(
    "https://h/v1?key=abcdef1234",
    r#"{"Authorization":"Bearer sk-abcdef","x-trace":"t1"}"#,
    None,
    RawBody::Text("{'model': 'x'}".to_owned()),
    &format!("{PREFIX}https://h/v1?key=*****1234 \\\n-H 'Authorization: Be****ef' -H 'x-trace: t1' \\\n-d '{{'model': 'x'}}'\n")
)]
#[case::no_headers_skips_the_header_line(
    "https://h/v1",
    "{}",
    None,
    RawBody::Text("body".to_owned()),
    &format!("{PREFIX}https://h/v1 \\\n-d 'body'\n")
)]
#[case::object_body_is_json_text(
    "https://h/v1",
    "{}",
    None,
    RawBody::Object(IndexMap::from([("a".to_owned(), Json::Bool(true))])),
    &format!("{PREFIX}https://h/v1 \\\n-d '{{\"a\": true}}'\n")
)]
#[case::non_text_header_value(
    "https://h/v1",
    r#"{"x-count":3}"#,
    None,
    RawBody::Text("body".to_owned()),
    &format!("{PREFIX}https://h/v1 \\\n-H 'x-count: 3' \\\n-d 'body'\n")
)]
#[case::request_str_replaces_the_command(
    "https://h/v1",
    r#"{"Authorization":"Bearer sk-abcdef"}"#,
    Some("sagemaker request"),
    RawBody::Text("body".to_owned()),
    "\nRequest Sent from LiteLLM:\nsagemaker request"
)]
fn logging_pre_post_behavior_tests_behavior(
    #[case] api_base: &str,
    #[case] headers_json: &str,
    #[case] request_str: Option<&str>,
    #[case] data: RawBody,
    #[case] expected: &str,
) {
    let headers: IndexMap<String, Json> =
        serde_json::from_str(headers_json).expect("the fixture is valid");
    let command = curl_command(&CurlRequest {
        api_base,
        headers: &headers,
        request_str,
        data: &data,
    });
    assert_eq!(command, expected);
}

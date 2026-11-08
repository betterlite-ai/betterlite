use rstest::rstest;

use crate::fields::response::provider_response_id;

use super::decode;

#[rstest]
#[case::text_id(r#"{"id":"abc"}"#, Some("abc"))]
#[case::empty_text(r#"{"id":""}"#, None)]
#[case::number_id(r#"{"id":5}"#, None)]
#[case::missing_id("{}", None)]
#[case::text_source(r#""abc""#, None)]
#[case::list_source(r#"["abc"]"#, None)]
#[case::null_source("null", None)]
fn logging_fields_behavior_tests_behavior(#[case] source: &str, #[case] expected: Option<&str>) {
    assert_eq!(
        provider_response_id(&decode(source)),
        expected.map(str::to_owned)
    );
}

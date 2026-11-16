#![cfg(test)]

use rstest::rstest;

use super::without_credentials;
use crate::straiker::v3::fixtures::parsed;

#[rstest]
#[case::mcp_server_credentials(
    r#"[{"type":"url","url":"https://mcp","authorization_token":"secret","name":"m"}]"#,
    r#"[{"type":"url","url":"https://mcp","authorization_token":"[redacted]","name":"m"}]"#
)]
#[case::authorization_and_headers(
    r#"[{"authorization":"Bearer x","headers":{"X-Key":"k"},"url":"u"}]"#,
    r#"[{"authorization":"[redacted]","headers":"[redacted]","url":"u"}]"#
)]
#[case::field_names_match_in_any_case(
    r#"[{"Authorization":"a","AUTHORIZATION_TOKEN":"b","Headers":"c"}]"#,
    r#"[{"Authorization":"[redacted]","AUTHORIZATION_TOKEN":"[redacted]","Headers":"[redacted]"}]"#
)]
#[case::every_entry_is_scrubbed(
    r#"[{"headers":"a"},{"name":"keep"},{"authorization":"b"}]"#,
    r#"[{"headers":"[redacted]"},{"name":"keep"},{"authorization":"[redacted]"}]"#
)]
#[case::schema_properties_are_not_touched(
    r#"[{"type":"function","function":{"name":"f","parameters":{"properties":{"headers":{"type":"string"},"token":{"type":"string"}}}}}]"#,
    r#"[{"type":"function","function":{"name":"f","parameters":{"properties":{"headers":{"type":"string"},"token":{"type":"string"}}}}}]"#
)]
#[case::nested_credentials_are_not_touched(
    r#"[{"server":{"authorization_token":"deep"}}]"#,
    r#"[{"server":{"authorization_token":"deep"}}]"#
)]
#[case::similar_field_names_are_kept(
    r#"[{"authorization_header":"a","header":"b","token":"c"}]"#,
    r#"[{"authorization_header":"a","header":"b","token":"c"}]"#
)]
#[case::non_object_entries_pass_through(
    r#"["x",3,null,[{"headers":"a"}]]"#,
    r#"["x",3,null,[{"headers":"a"}]]"#
)]
#[case::empty_list("[]", "[]")]
#[case::text_is_returned_as_is(r#""[{\"headers\":1}]""#, r#""[{\"headers\":1}]""#)]
#[case::object_is_returned_as_is(r#"{"headers":"a"}"#, r#"{"headers":"a"}"#)]
fn redacts_one_level(#[case] entries: &str, #[case] expected: &str) {
    assert_eq!(without_credentials(&parsed(entries)), parsed(expected));
}

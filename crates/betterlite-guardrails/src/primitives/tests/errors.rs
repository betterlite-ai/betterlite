use std::collections::BTreeMap;

use super::{FakeTransport, json_stringify, run};

enum Check {
    Returns(&'static str, &'static str, &'static str, &'static str),
    Fails(&'static str, &'static str, &'static str, &'static str),
    Unencodable,
}

#[rstest::rstest]
#[case::match_with_an_invalid_pattern(Check::Returns(
    "regex_match",
    r#"["text","[invalid"]"#,
    "{}",
    "false"
))]
#[case::match_all_with_an_invalid_pattern(Check::Returns(
    "regex_match_all",
    r#"["text","[invalid"]"#,
    "{}",
    "false"
))]
#[case::replace_keeps_the_original_text(Check::Returns(
    "regex_replace",
    r#"["text","[invalid","x"]"#,
    "{}",
    r#""text""#
))]
#[case::replace_with_a_bad_template_keeps_the_text(Check::Returns(
    "regex_replace",
    r#"["a","a","\\q"]"#,
    "{}",
    r#""a""#
))]
#[case::replace_with_a_missing_group_keeps_the_text(Check::Returns(
    "regex_replace",
    r#"["a","a","\\2"]"#,
    "{}",
    r#""a""#
))]
#[case::find_all_with_an_invalid_pattern(Check::Returns(
    "regex_find_all",
    r#"["text","[invalid"]"#,
    "{}",
    "[]"
))]
#[case::json_parse_of_broken_text(Check::Returns("json_parse", r#"["{nope"]"#, "{}", "null"))]
#[case::json_parse_of_empty_text(Check::Returns("json_parse", r#"[""]"#, "{}", "null"))]
#[case::json_parse_of_a_non_string(Check::Fails(
    "json_parse",
    "[5]",
    "{}",
    "primitive json_parse got arguments of the wrong shape"
))]
#[case::schema_with_a_wrong_keyword_type(Check::Returns(
    "json_schema_valid",
    r#"[{},{"type":5}]"#,
    "{}",
    "false"
))]
#[case::schema_that_is_not_an_object(Check::Returns(
    "json_schema_valid",
    r#"[{},"object"]"#,
    "{}",
    "false"
))]
#[case::stringify_of_an_unencodable_object(Check::Unencodable)]
#[case::too_many_positional_arguments(Check::Fails(
    "lower",
    r#"["a","b"]"#,
    "{}",
    "primitive lower takes 1 arguments but got 2"
))]
#[case::argument_given_twice(Check::Fails(
    "lower",
    r#"["a"]"#,
    r#"{"text":"b"}"#,
    "primitive lower got the argument text twice"
))]
#[case::unknown_named_argument(Check::Fails(
    "lower",
    r#"["a"]"#,
    r#"{"extra":1}"#,
    "primitive lower got arguments of the wrong shape"
))]
#[case::missing_required_argument(Check::Fails(
    "block",
    "[]",
    "{}",
    "primitive block got arguments of the wrong shape"
))]
#[case::allow_takes_no_arguments(Check::Fails(
    "allow",
    "[1]",
    "{}",
    "primitive allow takes 0 arguments but got 1"
))]
#[tokio::test]
async fn primitives_management_error(#[case] check: Check) {
    match check {
        Check::Returns(name, positional, named, expected) => {
            let output = run(FakeTransport::default(), name, positional, named)
                .await
                .expect("the call succeeds");
            assert_eq!(output, expected);
        }
        Check::Fails(name, positional, named, message) => {
            let error = run(FakeTransport::default(), name, positional, named)
                .await
                .expect_err("the call fails");
            assert_eq!(error.to_string(), message);
        }
        Check::Unencodable => {
            // JSON object keys must be strings, so a tuple key cannot be encoded.
            let tuple_keys = BTreeMap::from([((1_u8, 2_u8), 3_u8)]);
            assert_eq!(json_stringify(&tuple_keys), "");
        }
    }
}

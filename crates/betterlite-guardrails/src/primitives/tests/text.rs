use super::{FakeTransport, run};

#[rstest::rstest]
#[case::contains_found("contains", r#"["hello world","world"]"#, "true")]
#[case::contains_is_case_sensitive("contains", r#"["Hello","hello"]"#, "false")]
#[case::contains_any_one_match("contains_any", r#"["hello world",["world","missing"]]"#, "true")]
#[case::contains_any_no_match("contains_any", r#"["hello world",["x","y"]]"#, "false")]
#[case::contains_any_empty_list("contains_any", r#"["hello world",[]]"#, "false")]
#[case::contains_any_is_case_sensitive("contains_any", r#"["hello",["HELLO"]]"#, "false")]
#[case::contains_all_every_match("contains_all", r#"["hello world",["hello","world"]]"#, "true")]
#[case::contains_all_one_missing("contains_all", r#"["hello world",["hello","missing"]]"#, "false")]
#[case::contains_all_empty_list("contains_all", r#"["hello world",[]]"#, "true")]
#[case::word_count_splits_on_whitespace("word_count", r#"["  hello   big world \n"]"#, "3")]
#[case::word_count_splits_on_python_whitespace("word_count", r#"["a\u001fb c"]"#, "3")]
#[case::word_count_of_empty_text("word_count", r#"[""]"#, "0")]
#[case::char_count_counts_characters("char_count", "[\"h\u{e9}llo\"]", "5")]
#[case::char_count_of_empty_text("char_count", r#"[""]"#, "0")]
#[case::lower("lower", r#"["HeLLo"]"#, r#""hello""#)]
#[case::upper("upper", r#"["HeLLo"]"#, r#""HELLO""#)]
#[case::upper_expands_sharp_s("upper", "[\"stra\u{df}e\"]", r#""STRASSE""#)]
#[case::trim_both_ends("trim", r#"["  x y \n"]"#, r#""x y""#)]
#[case::trim_strips_python_whitespace("trim", r#"["\u001cx\u001f"]"#, r#""x""#)]
#[tokio::test]
async fn primitives_text_utilities(
    #[case] name: &str,
    #[case] positional: &str,
    #[case] expected: &str,
) {
    let output = run(FakeTransport::default(), name, positional, "{}")
        .await
        .expect("the call succeeds");
    assert_eq!(output, expected);
}

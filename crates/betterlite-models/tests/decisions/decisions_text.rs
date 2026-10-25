use betterlite_models::decisions::DecisionsJson;
use betterlite_models::decisions::text::decisions_text;
use rstest::rstest;

#[rstest]
#[case::text_passes_through(r#""a \"ticket\"""#, r#"a "ticket""#)]
#[case::object_spacing(
    r#"{"b":1,"a":[true,null,2.5]}"#,
    r#"{"b": 1, "a": [true, null, 2.5]}"#
)]
#[case::list_spacing(r#"[{"k":"v"},"x"]"#, r#"[{"k": "v"}, "x"]"#)]
#[case::ascii_escapes(
    "[\"caf\u{e9} \u{1f600}\u{7f}\\n\"]",
    "[\"caf\\u00e9 \\ud83d\\ude00\\u007f\\n\"]"
)]
fn writes_json_like_python(#[case] json: &str, #[case] text: &str) {
    let value: DecisionsJson = serde_json::from_str(json).unwrap();
    assert_eq!(decisions_text(&value).unwrap(), text);
}

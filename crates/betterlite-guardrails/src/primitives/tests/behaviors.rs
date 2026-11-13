use super::{FakeTransport, run};

#[rstest::rstest]
#[case::allow("allow", "[]", "{}", r#"{"action":"allow"}"#)]
#[case::block_reason_only("block", r#"["pii"]"#, "{}", r#"{"action":"block","reason":"pii"}"#)]
#[case::block_with_detection_info(
    "block",
    r#"["pii",{"kind":"ssn"}]"#,
    "{}",
    r#"{"action":"block","reason":"pii","detection_info":{"kind":"ssn"}}"#
)]
#[case::block_drops_empty_detection_info(
    "block",
    r#"["pii",{}]"#,
    "{}",
    r#"{"action":"block","reason":"pii"}"#
)]
#[case::block_drops_null_detection_info(
    "block",
    r#"["pii"]"#,
    r#"{"detection_info":null}"#,
    r#"{"action":"block","reason":"pii"}"#
)]
#[case::flag_defaults_metadata_to_empty(
    "flag",
    r#"["style"]"#,
    "{}",
    r#"{"action":"flag","reason":"style","metadata":{}}"#
)]
#[case::flag_with_named_metadata(
    "flag",
    r#"["style"]"#,
    r#"{"metadata":{"score":0.5}}"#,
    r#"{"action":"flag","reason":"style","metadata":{"score":0.5}}"#
)]
#[case::modify_with_nothing("modify", "[]", "{}", r#"{"action":"modify"}"#)]
#[case::modify_texts(
    "modify",
    r#"[["a","b"]]"#,
    "{}",
    r#"{"action":"modify","texts":["a","b"]}"#
)]
#[case::modify_keeps_an_explicit_empty_list(
    "modify",
    "[[]]",
    "{}",
    r#"{"action":"modify","texts":[]}"#
)]
#[case::modify_all_three(
    "modify",
    r#"[["x"],[{"url":"u"}]]"#,
    r#"{"tool_calls":[{"id":"1"}]}"#,
    r#"{"action":"modify","texts":["x"],"images":[{"url":"u"}],"tool_calls":[{"id":"1"}]}"#
)]
#[tokio::test]
async fn primitives_management_behaviors(
    #[case] name: &str,
    #[case] positional: &str,
    #[case] named: &str,
    #[case] expected: &str,
) {
    let output = run(FakeTransport::default(), name, positional, named)
        .await
        .expect("the call succeeds");
    assert_eq!(output, expected);
}

use super::{FakeTransport, run};

#[rstest::rstest]
#[case::detect_code_python("detect_code", r#"["def foo(): pass"]"#, "true")]
#[case::detect_code_plain_text("detect_code", r#"["plain text"]"#, "false")]
#[case::detect_code_empty_text("detect_code", r#"[""]"#, "false")]
#[case::languages_python("detect_code_languages", r#"["def foo(): pass"]"#, r#"["python"]"#)]
#[case::languages_sql(
    "detect_code_languages",
    r#"["SELECT name FROM users WHERE id = 1"]"#,
    r#"["sql"]"#
)]
#[case::languages_html(
    "detect_code_languages",
    r#"["<div class=\"a\">hi</div>"]"#,
    r#"["html"]"#
)]
#[case::languages_javascript(
    "detect_code_languages",
    r#"["console.log('x')"]"#,
    r#"["javascript"]"#
)]
#[case::languages_rust(
    "detect_code_languages",
    r#"["fn main() { println!(\"hi\"); }"]"#,
    r#"["rust"]"#
)]
#[case::languages_shell(
    "detect_code_languages",
    r##"["#!/bin/bash\necho hi"]"##,
    r#"["shell"]"#
)]
#[case::languages_css("detect_code_languages", r#"["a { color: red; }"]"#, r#"["css"]"#)]
#[case::languages_go("detect_code_languages", r#"["x := 1"]"#, r#"["go"]"#)]
#[case::languages_java(
    "detect_code_languages",
    r#"["public static void main"]"#,
    r#"["java"]"#
)]
#[case::languages_typescript("detect_code_languages", r#"["a: string"]"#, r#"["typescript"]"#)]
#[case::languages_keep_table_order(
    "detect_code_languages",
    r#"["let x: string"]"#,
    r#"["javascript","typescript","rust"]"#
)]
#[case::languages_none("detect_code_languages", r#"["hello there"]"#, "[]")]
#[case::contains_language_ignores_case(
    "contains_code_language",
    r#"["SELECT a FROM t",["SQL"]]"#,
    "true"
)]
#[case::contains_language_other_language(
    "contains_code_language",
    r#"["SELECT a FROM t",["python"]]"#,
    "false"
)]
#[case::contains_language_any_of_many(
    "contains_code_language",
    r#"["SELECT a FROM t",["python","sql"]]"#,
    "true"
)]
#[case::contains_language_empty_list(
    "contains_code_language",
    r#"["SELECT a FROM t",[]]"#,
    "false"
)]
#[tokio::test]
async fn primitives_code_detection(
    #[case] name: &str,
    #[case] positional: &str,
    #[case] expected: &str,
) {
    let output = run(FakeTransport::default(), name, positional, "{}")
        .await
        .expect("the call succeeds");
    assert_eq!(output, expected);
}

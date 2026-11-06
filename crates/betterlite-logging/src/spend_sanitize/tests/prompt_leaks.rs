use crate::spend_sanitize::prompt_leaks::redact_spend_error_text;

const REDACTED: &str = "\"REDACTED_BY_LITELM\"";

fn redacts(source: &str, expected: &str) {
    assert_eq!(redact_spend_error_text(source), expected);
}

#[test]
fn redacts_nested_structured_values_under_a_quoted_key() {
    redacts(
        r#"{"input": [{"content": "secret"}], "x": 1}"#,
        &format!(r#"{{"input": {REDACTED}, "x": 1}}"#),
    );
    redacts(
        "'prompt': 'hello' there",
        &format!("'prompt': {REDACTED} there"),
    );
    redacts(
        r#""input": {"a": "x]y"} tail"#,
        &format!(r#""input": {REDACTED} tail"#),
    );
    redacts(r#"x"input": "a"y"#, &format!(r#"x"input": {REDACTED}y"#));
}

#[test]
fn redacts_assignment_style_pydantic_values() {
    redacts(
        "input_value={'a': 1} rest",
        &format!("input_value={REDACTED} rest"),
    );
}

#[test]
fn leaves_unquoted_scalars_intact_and_keeps_scanning() {
    redacts(
        r#""messages": 42 and "input": {"a": 1}"#,
        &format!(r#""messages": 42 and "input": {REDACTED}"#),
    );
    redacts(
        r#"prefix "input": null suffix"#,
        r#"prefix "input": null suffix"#,
    );
    redacts("myinput_value=1", "myinput_value=1");
    redacts("my_input_value=1", "my_input_value=1");
}

// A combining mark, a symbol and a letter are the boundary shapes Python's \w distinguishes
#[test]
fn redacts_after_a_character_python_does_not_count_as_a_word_character() {
    redacts(
        "\u{93e}input_value={'a': 1}",
        &format!("\u{93e}input_value={REDACTED}"),
    );
    redacts(
        "\u{345}input_value={'a': 1}",
        &format!("\u{345}input_value={REDACTED}"),
    );
    redacts(
        "\u{24b6}input_value={'a': 1}",
        &format!("\u{24b6}input_value={REDACTED}"),
    );
    redacts("\u{e9}input_value={'a': 1}", "\u{e9}input_value={'a': 1}");
}

#[test]
fn honors_escaped_quotes_inside_the_redacted_value() {
    redacts(
        r#""input": "a\"b" tail"#,
        &format!(r#""input": {REDACTED} tail"#),
    );
    redacts(
        r#""input": "a\\" tail"#,
        &format!(r#""input": {REDACTED} tail"#),
    );
}

#[test]
fn redacts_through_the_end_of_an_unterminated_value() {
    redacts(r#""input": [1, 2"#, &format!(r#""input": {REDACTED}"#));
    redacts(
        r#""input": 'unterminated"#,
        &format!(r#""input": {REDACTED}"#),
    );
    redacts(r#""input": "a\" tail"#, &format!(r#""input": {REDACTED}"#));
}

#[test]
fn keeps_a_key_that_has_no_value() {
    redacts(r#""input":"#, r#""input":"#);
    redacts("", "");
}

#[test]
fn keeps_the_source_spacing_around_the_separator() {
    redacts(
        r#""input"  :  [1] done"#,
        &format!(r#""input"  :  {REDACTED} done"#),
    );
    redacts(r#""input': [1]"#, &format!(r#""input': {REDACTED}"#));
}

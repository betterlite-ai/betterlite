#[rstest]
#[case::lower_true("true", Some(true))]
#[case::mixed_false("False", Some(false))]
#[case::padded(" False ", Some(false))]
#[case::tabbed("\ttrue", Some(true))]
#[case::yes("yes", None)]
#[case::one("1", None)]
#[case::on("on", None)]
#[case::empty("", None)]
fn env_parses_true_and_false_with_text_to_bool(#[case] text: &str, #[case] expected: Option<bool>) {
    let environment = environment(&[("FLAG", text)]);
    let value = base_lookup()
        .get_secret(&environment, "FLAG", None)
        .expect("the environment branch should not fail");
    assert_eq!(bool_value(value.as_ref()), expected);
}

#[rstest]
#[case::yes("yes")]
#[case::one("1")]
#[case::on("on")]
#[case::sentence("true enough")]
fn env_keeps_other_text_as_text(#[case] text: &str) {
    let environment = environment(&[("FLAG", text)]);
    let value = base_lookup()
        .get_secret(&environment, "FLAG", None)
        .expect("the environment branch should not fail");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some(text));
    assert_eq!(bool_value(value.as_ref()), None);
}

#[test]
fn env_empty_value_is_empty_text() {
    let environment = environment(&[("EMPTY", "")]);
    let value = base_lookup()
        .get_secret(&environment, "EMPTY", None)
        .expect("an empty value is not an error");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some(""));
}

#[test]
fn env_missing_name_returns_none_even_with_a_default() {
    let lookup = base_lookup();
    let empty = BTreeMap::new();
    let text_default = SecretValue::Text(SecretString::from("fallback"));
    assert!(
        lookup
            .get_secret(&empty, "MISSING", Some(text_default))
            .expect("a missing name is not an error")
            .is_none()
    );
    let str_default = SecretValue::Text(SecretString::from("fallback"));
    assert!(
        lookup
            .get_secret_str(&empty, "MISSING", Some(str_default))
            .expect("a missing name is not an error")
            .is_none()
    );
    assert!(
        lookup
            .get_secret_bool(&empty, "MISSING", Some(true))
            .expect("a missing name is not an error")
            .is_none()
    );
}

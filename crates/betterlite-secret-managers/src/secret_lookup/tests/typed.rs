#[test]
fn typed_get_secret_str_returns_only_text() {
    let environment = environment(&[("TEXT", "plain"), ("FLAG", "TRUE")]);
    let lookup = base_lookup();
    let text = lookup
        .get_secret_str(&environment, "TEXT", None)
        .expect("a text value should resolve");
    assert_eq!(
        text.as_ref().map(ExposeSecret::expose_secret),
        Some("plain")
    );
    assert!(
        lookup
            .get_secret_str(&environment, "FLAG", None)
            .expect("a boolean value is not an error")
            .is_none()
    );
    assert!(
        lookup
            .get_secret_str(&environment, "MISSING", None)
            .expect("a missing name is not an error")
            .is_none()
    );
}

#[test]
fn typed_get_secret_bool_returns_bools_and_parses_text() {
    let environment = environment(&[("FLAG", "false"), ("WORD", "TRUE"), ("OTHER", "yes")]);
    let lookup = base_lookup();
    assert_eq!(
        lookup
            .get_secret_bool(&environment, "FLAG", None)
            .expect("a boolean value should resolve"),
        Some(false)
    );
    assert_eq!(
        lookup
            .get_secret_bool(&environment, "WORD", None)
            .expect("recognized text should parse"),
        Some(true)
    );
    assert_eq!(
        lookup
            .get_secret_bool(&environment, "OTHER", None)
            .expect("unrecognized text is not an error"),
        None
    );
    assert_eq!(
        lookup
            .get_secret_bool(&environment, "MISSING", None)
            .expect("a missing name is not an error"),
        None
    );
}

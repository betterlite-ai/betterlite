#[test]
fn prefix_removes_a_single_os_environ_reference() {
    let environment = environment(&[("DB_PASSWORD", "db-secret")]);
    let value = base_lookup()
        .get_secret(&environment, "os.environ/DB_PASSWORD", None)
        .expect("the lookup should succeed");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("db-secret"));
}

#[test]
fn prefix_removes_every_os_environ_occurrence() {
    let environment = environment(&[("DB_PASSWORD", "db-secret"), ("AB", "joined")]);
    let lookup = base_lookup();
    let repeated = lookup
        .get_secret(&environment, "os.environ/os.environ/DB_PASSWORD", None)
        .expect("two leading prefixes should resolve");
    assert_eq!(text_value(repeated.as_ref()).as_deref(), Some("db-secret"));
    let embedded = lookup
        .get_secret(&environment, "os.environ/Aos.environ/B", None)
        .expect("an embedded prefix should be removed too");
    assert_eq!(text_value(embedded.as_ref()).as_deref(), Some("joined"));
}

#[test]
fn prefix_leaves_a_name_that_starts_differently_alone() {
    let environment = environment(&[("prefix/os.environ/NAME", "literal")]);
    let value = base_lookup()
        .get_secret(&environment, "prefix/os.environ/NAME", None)
        .expect("the literal name should resolve");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("literal"));
}

#[test]
fn prefix_missing_name_returns_none() {
    let value = base_lookup()
        .get_secret(&BTreeMap::new(), "os.environ/MISSING", None)
        .expect("a missing name is not an error");
    assert!(value.is_none());
}

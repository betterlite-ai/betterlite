#[test]
fn secret_lookup_clone_keeps_the_manager_and_the_tokens() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let (mut lookup, http) = scripted_lookup(
        vec![
            answer("first-token"),
            answer("second-token"),
            answer("third-token"),
        ],
        &clock,
    );
    let manager = Arc::new(FakeManager::with(ManagerReply::Text("manager-value")));
    lookup.manager = Some(manager_state(
        custom_client_with(&manager),
        Some(SecretManagerSettings::default()),
    ));
    let first = lookup
        .get_secret(&BTreeMap::new(), "oidc/google/first", None)
        .expect("the first token read should succeed");
    assert_eq!(text_value(first.as_ref()).as_deref(), Some("first-token"));
    let copy = lookup.clone();
    let managed = copy
        .get_secret(&BTreeMap::new(), "db-password", None)
        .expect("the clone reads through the same manager");
    assert_eq!(
        text_value(managed.as_ref()).as_deref(),
        Some("manager-value")
    );
    assert_eq!(manager.reads(), vec!["db-password".to_owned()]);
    let cached = copy
        .get_secret(&BTreeMap::new(), "oidc/google/first", None)
        .expect("the clone carries the token fetched before it");
    assert_eq!(text_value(cached.as_ref()).as_deref(), Some("first-token"));
    assert_eq!(http.requests().len(), 1);
    let later = copy
        .get_secret(&BTreeMap::new(), "oidc/google/second", None)
        .expect("the clone fetches a token of its own");
    assert_eq!(text_value(later.as_ref()).as_deref(), Some("second-token"));
    assert_eq!(http.requests().len(), 2);
    let refetched = lookup
        .get_secret(&BTreeMap::new(), "oidc/google/second", None)
        .expect("the original does not see the clone's new token");
    assert_eq!(
        text_value(refetched.as_ref()).as_deref(),
        Some("third-token")
    );
    assert_eq!(http.requests().len(), 3);
}

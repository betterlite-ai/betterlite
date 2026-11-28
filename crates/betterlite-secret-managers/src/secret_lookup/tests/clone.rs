#[test]
fn secret_lookup_clone_keeps_the_manager_and_the_tokens() {
    let clock = Arc::new(ManualClock::at(1_700_000_000));
    let replies = vec![
        answer("first-token"),
        answer("second-token"),
        answer("third-token"),
    ];
    let (mut lookup, http) = scripted_lookup(replies, &clock);
    let manager = Arc::new(FakeManager::with(ManagerReply::Text("manager-value")));
    lookup.manager = Some(manager_state(
        custom_client_with(&manager),
        Some(SecretManagerSettings::default()),
    ));
    let fetched = lookup
        .get_secret(&BTreeMap::new(), "oidc/google/first", None)
        .expect("the first token read should succeed");
    assert_eq!(text_value(fetched.as_ref()).as_deref(), Some("first-token"));
    let mirror = lookup.clone();
    let read = mirror
        .get_secret(&BTreeMap::new(), "db-password", None)
        .expect("the clone reads through the same manager");
    assert_eq!(text_value(read.as_ref()).as_deref(), Some("manager-value"));
    assert_eq!(manager.reads(), vec!["db-password".to_owned()]);
    let carried = mirror
        .get_secret(&BTreeMap::new(), "oidc/google/first", None)
        .expect("the clone carries the token fetched before it");
    assert_eq!(text_value(carried.as_ref()).as_deref(), Some("first-token"));
    assert_eq!(http.requests().len(), 1);
    let later = mirror
        .get_secret(&BTreeMap::new(), "oidc/google/second", None)
        .expect("the clone fetches a token of its own");
    assert_eq!(text_value(later.as_ref()).as_deref(), Some("second-token"));
    let origin = lookup
        .get_secret(&BTreeMap::new(), "oidc/google/second", None)
        .expect("the original does not see the clone's new token");
    assert_eq!(text_value(origin.as_ref()).as_deref(), Some("third-token"));
    assert_eq!(http.requests().len(), 3);
}

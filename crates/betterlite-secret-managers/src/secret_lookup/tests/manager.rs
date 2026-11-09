#[test]
fn manager_reads_a_text_value_through_the_client() {
    let manager = Arc::new(FakeManager::with(ManagerReply::Text("manager-value")));
    let mut lookup = base_lookup();
    lookup.manager = Some(manager_state(
        custom_client_with(&manager),
        Some(SecretManagerSettings::default()),
    ));
    let value = lookup
        .get_secret(&BTreeMap::new(), "db-password", None)
        .expect("the manager read should succeed");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("manager-value"));
    assert_eq!(manager.reads(), vec!["db-password".to_owned()]);
}

#[rstest]
#[case::padded_true("  True\n", Some(true), None)]
#[case::plain_false("False", Some(false), None)]
#[case::trailing_space("True ", Some(true), None)]
#[case::lowercase_true("true", None, Some("true"))]
#[case::commented("True # comment", None, Some("True # comment"))]
#[case::parenthesized("(True)", None, Some("(True)"))]
#[case::number("1", None, Some("1"))]
fn manager_text_becomes_a_boolean_only_for_the_python_literals(
    #[case] text: &'static str,
    #[case] expected_bool: Option<bool>,
    #[case] expected_text: Option<&str>,
) {
    let mut lookup = base_lookup();
    lookup.manager = Some(manager_state(
        custom_client(ManagerReply::Text(text)),
        Some(SecretManagerSettings::default()),
    ));
    let value = lookup
        .get_secret(&BTreeMap::new(), "db-password", None)
        .expect("the manager read should succeed");
    assert_eq!(bool_value(value.as_ref()), expected_bool);
    assert_eq!(text_value(value.as_ref()).as_deref(), expected_text);
}

#[test]
fn manager_dispatch_error_falls_back_to_the_environment() {
    let (logs, _guard) = capture_logs();
    let mut lookup = base_lookup();
    lookup.manager = Some(manager_state(
        custom_client(ManagerReply::Failure),
        Some(SecretManagerSettings::default()),
    ));
    let environment = environment(&[("db-password", "True")]);
    let value = lookup
        .get_secret(&environment, "db-password", None)
        .expect("the environment fallback should succeed");
    assert_eq!(bool_value(value.as_ref()), Some(true));
    assert!(
        logs.text().contains(
            "Defaulting to os.environ value for key=db-password. An exception occurred - "
        )
    );
}

#[test]
fn manager_dispatch_error_without_an_environment_value_returns_none() {
    let mut lookup = base_lookup();
    lookup.manager = Some(manager_state(
        custom_client(ManagerReply::Failure),
        Some(SecretManagerSettings::default()),
    ));
    let value = lookup
        .get_secret(&BTreeMap::new(), "db-password", None)
        .expect("a missing fallback is not an error");
    assert!(value.is_none());
}

#[test]
fn manager_none_result_does_not_fall_back_to_the_environment() {
    let mut state = manager_state(
        aws_secrets_client(None),
        Some(SecretManagerSettings::default()),
    );
    state.system = Some(KeyManagementSystem::AwsSecretManager);
    let mut lookup = base_lookup();
    lookup.manager = Some(state);
    let environment = environment(&[("db-password", "env-value")]);
    let value = lookup
        .get_secret(&environment, "db-password", None)
        .expect("a None dispatch result is not an error");
    assert!(value.is_none());
}

#[test]
fn manager_hosted_keys_excluding_the_name_reads_the_environment_locally() {
    let manager = Arc::new(FakeManager::with(ManagerReply::Text("manager-value")));
    let mut state = manager_state(
        custom_client_with(&manager),
        Some(SecretManagerSettings::default()),
    );
    state.hosted_keys = Some(vec!["other-key".to_owned()]);
    let mut lookup = base_lookup();
    lookup.manager = Some(state);
    let variables = environment(&[("db-password", "true")]);
    let literal = lookup
        .get_secret(&variables, "db-password", None)
        .expect("the local read should succeed");
    assert_eq!(text_value(literal.as_ref()).as_deref(), Some("true"));
    assert_eq!(manager.reads(), Vec::<String>::new());
    let upper_variables = environment(&[("db-password", "True")]);
    let parsed = lookup
        .get_secret(&upper_variables, "db-password", None)
        .expect("the local read should succeed");
    assert_eq!(bool_value(parsed.as_ref()), Some(true));
}

#[test]
fn manager_hosted_keys_including_the_name_reads_the_manager() {
    let mut state = manager_state(
        custom_client(ManagerReply::Text("manager-value")),
        Some(SecretManagerSettings::default()),
    );
    state.hosted_keys = Some(vec!["db-password".to_owned()]);
    let mut lookup = base_lookup();
    lookup.manager = Some(state);
    let value = lookup
        .get_secret(&BTreeMap::new(), "db-password", None)
        .expect("the manager read should succeed");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("manager-value"));
}

#[test]
fn manager_hosted_keys_match_the_name_after_the_prefix_is_removed() {
    let mut state = manager_state(
        custom_client(ManagerReply::Text("manager-value")),
        Some(SecretManagerSettings::default()),
    );
    state.hosted_keys = Some(vec!["db-password".to_owned()]);
    let mut lookup = base_lookup();
    lookup.manager = Some(state);
    let value = lookup
        .get_secret(&BTreeMap::new(), "os.environ/db-password", None)
        .expect("the manager read should succeed");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("manager-value"));
}

#[test]
fn manager_write_only_access_reads_the_environment() {
    let manager = Arc::new(FakeManager::with(ManagerReply::Text("manager-value")));
    let mut state = manager_state(
        custom_client_with(&manager),
        Some(SecretManagerSettings::default()),
    );
    state.access_mode = SecretAccessMode::WriteOnly;
    let mut lookup = base_lookup();
    lookup.manager = Some(state);
    let environment = environment(&[("db-password", "true")]);
    let value = lookup
        .get_secret(&environment, "db-password", None)
        .expect("the environment branch should succeed");
    assert_eq!(bool_value(value.as_ref()), Some(true));
    assert_eq!(manager.reads(), Vec::<String>::new());
}

#[test]
fn manager_oidc_names_bypass_the_configured_manager() {
    let manager = Arc::new(FakeManager::with(ManagerReply::Text("manager-value")));
    let mut state = manager_state(
        custom_client_with(&manager),
        Some(SecretManagerSettings::default()),
    );
    state.hosted_keys = Some(vec!["oidc/env/TOKEN".to_owned()]);
    let mut lookup = base_lookup();
    lookup.manager = Some(state);
    let variables = environment(&[("TOKEN", "env-token")]);
    let value = lookup
        .get_secret(&variables, "oidc/env/TOKEN", None)
        .expect("the OIDC provider should succeed");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("env-token"));
    assert_eq!(manager.reads(), Vec::<String>::new());
}

#[test]
fn manager_without_settings_reads_the_environment() {
    let mut lookup = base_lookup();
    lookup.manager = Some(manager_state(custom_client(ManagerReply::Failure), None));
    let environment = environment(&[("db-password", "env-value")]);
    let value = lookup
        .get_secret(&environment, "db-password", None)
        .expect("the environment branch should succeed");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("env-value"));
}

#[test]
fn manager_without_a_system_reads_the_environment_without_a_dispatch_error() {
    let (logs, _guard) = capture_logs();
    let mut state = manager_state(
        custom_client(ManagerReply::Text("manager-value")),
        Some(SecretManagerSettings::default()),
    );
    state.system = None;
    let mut lookup = base_lookup();
    lookup.manager = Some(state);
    let environment = environment(&[("db-password", "env-value")]);
    let value = lookup
        .get_secret(&environment, "db-password", None)
        .expect("the local read should succeed");
    assert_eq!(text_value(value.as_ref()).as_deref(), Some("env-value"));
    assert!(!logs.text().contains("Defaulting to os.environ value"));
}

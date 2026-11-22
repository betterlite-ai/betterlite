fn enabled_harness() -> Harness {
    let mut enabled = harness();
    enabled.prefixes.short_prefix_enabled = true;
    enabled
}

#[test]
fn prefix_assigns_the_natural_hash_when_free() {
    let harness = enabled_harness();
    let mut subject = server(r#"{"server_id":"srv-1","name":"srv-1","transport":"http"}"#);
    let taken = BTreeMap::new();
    let (captured, _guard) = capture_at(Level::INFO);
    let result = harness
        .builder()
        .assign_unique_short_prefix(&mut subject, &taken);
    result.unwrap();
    let expected = compute_short_server_prefix("srv-1", 0).unwrap();
    assert_eq!(subject.fields.short_prefix, Some(expected));
    assert!(!captured.text().contains("collision resolved"));
}

#[test]
fn prefix_config_stdio_policy_mirrors_the_enabled_flag() {
    let mut policy = ConfigStdioPolicy {
        enabled: false,
        allowed_commands: BTreeSet::new(),
    };
    assert!(<ConfigStdioPolicy as StdioPolicy>::is_blocked(&policy));
    policy.enabled = true;
    assert!(!<ConfigStdioPolicy as StdioPolicy>::is_blocked(&policy));
}

#[test]
fn prefix_keeps_an_existing_prefix() {
    let harness = enabled_harness();
    let source = r#"{"server_id":"srv-1","name":"srv-1","transport":"http","short_prefix":"XYZ"}"#;
    let mut subject = server(source);
    let (captured, _guard) = capture_at(Level::INFO);
    let taken = BTreeMap::new();
    let result = harness
        .builder()
        .assign_unique_short_prefix(&mut subject, &taken);
    result.unwrap();
    assert_eq!(subject.fields.short_prefix.as_deref(), Some("XYZ"));
    assert!(captured.text().is_empty());
}

#[test]
fn prefix_rehashes_past_a_collision() {
    let harness = enabled_harness();
    let natural = compute_short_server_prefix("srv-1", 0).unwrap();
    let taken = BTreeMap::from([(natural, String::from("other-server"))]);
    let (captured, _guard) = capture_at(Level::INFO);
    let mut subject = server(r#"{"server_id":"srv-1","name":"srv-1","transport":"http"}"#);
    let result = harness
        .builder()
        .assign_unique_short_prefix(&mut subject, &taken);
    result.unwrap();
    let expected = compute_short_server_prefix("srv-1", 1).unwrap();
    assert_eq!(subject.fields.short_prefix, Some(expected));
    let text = captured.text();
    assert!(text.contains("collided with other-server"));
    assert!(text.contains("attempt=1"));
}

#[test]
fn prefix_skips_its_own_entry() {
    let harness = enabled_harness();
    let natural = compute_short_server_prefix("srv-1", 0).unwrap();
    let taken = BTreeMap::from([(natural.clone(), String::from("srv-1"))]);
    let mut subject = server(r#"{"server_id":"srv-1","name":"srv-1","transport":"http"}"#);
    let result = harness
        .builder()
        .assign_unique_short_prefix(&mut subject, &taken);
    result.unwrap();
    assert_eq!(subject.fields.short_prefix, Some(natural));
}

#[test]
fn prefix_does_nothing_when_disabled() {
    let harness = harness();
    let mut subject = server(r#"{"server_id":"srv-1","name":"srv-1","transport":"http"}"#);
    let taken = BTreeMap::new();
    let result = harness
        .builder()
        .assign_unique_short_prefix(&mut subject, &taken);
    result.unwrap();
    assert!(subject.fields.short_prefix.is_none());
}

#[test]
fn prefix_errors_after_1024_attempts() {
    let harness = enabled_harness();
    let taken = all_short_prefixes("srv-1");
    let mut subject = server(r#"{"server_id":"srv-1","name":"srv-1","transport":"http"}"#);
    let error = harness
        .builder()
        .assign_unique_short_prefix(&mut subject, &taken)
        .unwrap_err();
    assert!(matches!(
        &error,
        BuildError::PrefixExhausted { attempts: 1024, .. }
    ));
    assert_eq!(
        error.to_string(),
        "Unable to assign a unique short MCP tool prefix for server srv-1 after 1024 attempts; the 3-character prefix space is too crowded."
    );
}

#[test]
fn prefix_stdio_warns_on_a_new_block() {
    let mut harness = harness();
    harness.stdio = FakeStdioPolicy(true);
    let (captured, _guard) = capture();
    let table =
        row(r#"{"server_id":"srv-1","transport":"stdio","alias":"test-stdio","command":"npx"}"#);
    harness.builder().warn_if_newly_blocked_stdio(&table, None);
    let text = captured.text();
    assert!(text.contains("MCP server 'test-stdio' will not start:"));
    assert_eq!(text.matches(MCP_STDIO_DISABLED_MESSAGE).count(), 1);
}

#[test]
fn prefix_stdio_warns_when_the_transport_changes() {
    let mut harness = harness();
    harness.stdio = FakeStdioPolicy(true);
    let previous = server(r#"{"server_id":"srv-1","name":"test","transport":"http"}"#);
    let table = row(r#"{"server_id":"srv-1","transport":"stdio","server_name":"test-stdio"}"#);
    let (captured, _guard) = capture();
    harness
        .builder()
        .warn_if_newly_blocked_stdio(&table, Some(&previous));
    assert!(
        captured
            .text()
            .contains("MCP server 'test-stdio' will not start:")
    );
}

#[test]
fn prefix_stdio_is_silent_when_already_blocked() {
    let mut blocked = harness();
    blocked.stdio = FakeStdioPolicy(true);
    let previous = server(r#"{"server_id":"srv-1","name":"test","transport":"stdio"}"#);
    let table = row(r#"{"server_id":"srv-1","transport":"stdio","alias":"test-stdio"}"#);
    let http_table = row(r#"{"server_id":"srv-2","transport":"http"}"#);
    let (captured, _guard) = capture();
    blocked
        .builder()
        .warn_if_newly_blocked_stdio(&table, Some(&previous));
    blocked
        .builder()
        .warn_if_newly_blocked_stdio(&http_table, None);
    let mut unblocked = harness();
    unblocked.stdio = FakeStdioPolicy(false);
    unblocked
        .builder()
        .warn_if_newly_blocked_stdio(&table, None);
    assert!(captured.text().is_empty());
}

#[test]
fn merge_returns_a_clone_without_metadata() {
    let server = oauth_server();
    assert_eq!(merge_discovered(&server, None), server);
}

#[test]
fn merge_keeps_values_the_server_already_has() {
    let mut server = oauth_server();
    server.fields.scopes = Some(vec!["read".to_owned()]);
    server.fields.issuer = Some("https://idp.test".to_owned());
    server.fields.authorization_url = Some("https://idp.test/authorize".to_owned());
    server.fields.token_url = Some("https://idp.test/token".to_owned());
    server.fields.registration_url = Some("https://idp.test/register".to_owned());
    let metadata = discovered_metadata();
    assert_eq!(merge_discovered(&server, Some(&metadata)), server);
}

#[test]
fn merge_fills_missing_values_from_the_metadata() {
    let server = oauth_server();
    let metadata = discovered_metadata();
    let merged = merge_discovered(&server, Some(&metadata));
    assert_eq!(merged.fields.scopes, Some(vec!["read".to_owned()]));
    assert_eq!(merged.fields.issuer.as_deref(), Some("https://other.test"));
    assert_eq!(
        merged.fields.authorization_url.as_deref(),
        Some("https://other.test/authorize")
    );
    assert_eq!(
        merged.fields.token_url.as_deref(),
        Some("https://other.test/token")
    );
    assert_eq!(
        merged.fields.registration_url.as_deref(),
        Some("https://other.test/register")
    );
}

#[test]
fn merge_keeps_the_configured_issuer_over_the_discovered_one() {
    let mut server = oauth_server();
    server.fields.issuer = Some("https://idp.test".to_owned());
    let metadata = discovered_metadata();
    assert_eq!(
        merge_discovered(&server, Some(&metadata))
            .fields
            .issuer
            .as_deref(),
        Some("https://idp.test")
    );
}

#[test]
fn merge_drops_the_issuer_of_an_origin_fallback_document() {
    let server = oauth_server();
    let metadata = metadata_from_json(
        r#"{
            "authorization_url": "https://other.test/authorize",
            "discovered_issuer": "https://other.test",
            "from_origin_fallback": true
        }"#,
    );
    let merged = merge_discovered(&server, Some(&metadata));
    assert_eq!(merged.fields.issuer, None);
    assert_eq!(
        merged.fields.authorization_url.as_deref(),
        Some("https://other.test/authorize")
    );
}

#[test]
fn merge_treats_an_empty_stored_url_as_unset() {
    let mut server = oauth_server();
    server.fields.token_url = Some(String::new());
    let metadata = discovered_metadata();
    assert_eq!(
        merge_discovered(&server, Some(&metadata))
            .fields
            .token_url
            .as_deref(),
        Some("https://other.test/token")
    );
}

#[rstest]
#[case::spaced(" read  write ", Some(&["read", "write"][..]))]
#[case::tabs("read\twrite", Some(&["read", "write"][..]))]
#[case::empty("", None)]
#[case::whitespace("   ", None)]
fn merge_extract_scopes_splits_text_on_whitespace(
    #[case] text: &str,
    #[case] expected: Option<&[&str]>,
) {
    let wanted = expected.map(|values| {
        values
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<String>>()
    });
    assert_eq!(extract_scopes(ScopesInput::Text(text)), wanted);
}

#[test]
fn merge_extract_scopes_keeps_list_strings_untrimmed() {
    let values = vec![
        json_string(" read "),
        json_string(""),
        json_string("   "),
        JsonValue::Bool(true),
        JsonValue::Null,
        JsonValue::Number(serde_json::Number::from(7_i32)),
    ];
    assert_eq!(
        extract_scopes(ScopesInput::List(&values)),
        Some(vec![" read ".to_owned()])
    );
    let empty = vec![JsonValue::Null, json_string("  ")];
    assert_eq!(extract_scopes(ScopesInput::List(&empty)), None);
}

#[test]
fn merge_extract_scopes_returns_none_for_missing_input() {
    assert_eq!(extract_scopes(ScopesInput::Missing), None);
}

#[rstest]
#[case::both_needed_without_manual(true, true, None, None, true)]
#[case::authorization_configured(true, false, Some("https://idp.test/authorize"), None, false)]
#[case::token_not_needed(true, false, None, None, true)]
#[case::blank_manual(true, false, Some("   "), None, true)]
fn merge_discovery_leaves_unresolved_decides_per_needed_field(
    #[case] needs_authorization: bool,
    #[case] needs_token: bool,
    #[case] manual_authorization: Option<&str>,
    #[case] manual_token: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(
        discovery_leaves_unresolved(
            NeededEndpoint {
                needed: needs_authorization,
                manual: manual_authorization,
            },
            NeededEndpoint {
                needed: needs_token,
                manual: manual_token,
            },
        ),
        expected
    );
}

#[rstest]
#[case::deferred(true, false)]
#[case::anchored(false, true)]
fn merge_warn_unresolved_is_silent_when_deferred_or_anchored(
    #[case] deferred: bool,
    #[case] anchored: bool,
) {
    let (captured, _guard) = capture();
    let mut check = unresolved_check();
    check.discovery_deferred = deferred;
    check.issuer_anchored = anchored;
    warn_unresolved(check);
    assert_eq!(captured.text(), "");
}

#[test]
fn merge_warn_unresolved_after_discovery_names_fields_and_redacted_origin() {
    let (captured, _guard) = capture();
    warn_unresolved(unresolved_check());
    let text = captured.text();
    assert!(text.contains("authorization_url, token_url"));
    assert!(text.contains("https://mcp.test:8443"));
    assert!(!text.contains("secret"));
}

#[test]
fn merge_warn_unresolved_without_a_source_uses_is_and_are() {
    let (captured, _guard) = capture();
    let mut plural = unresolved_check();
    plural.discovery_attempted = false;
    warn_unresolved(plural);
    assert!(
        captured
            .text()
            .contains("authorization_url and token_url are not set manually")
    );
    let (single_captured, _single_guard) = capture();
    let mut single = unresolved_check();
    single.discovery_attempted = false;
    single.needs_token_url = false;
    warn_unresolved(single);
    assert!(
        single_captured
            .text()
            .contains("authorization_url is not set manually")
    );
}

#[rstest]
#[case::manual(Some("https://idp.test/authorize"), false, "")]
#[case::metadata(None, true, "")]
#[case::neither(None, false, "authorization_url")]
fn merge_warn_unresolved_accepts_manual_and_discovered_values(
    #[case] manual_authorization: Option<&str>,
    #[case] with_metadata: bool,
    #[case] expected: &str,
) {
    let metadata = discovered_metadata();
    let (captured, _guard) = capture();
    let mut check = unresolved_check();
    check.needs_token_url = false;
    check.manual_authorization_url = manual_authorization;
    check.metadata = with_metadata.then_some(&metadata);
    warn_unresolved(check);
    let text = captured.text();
    if expected.is_empty() {
        assert_eq!(text, "");
    } else {
        assert!(text.contains(expected));
    }
}

#[test]
fn merge_warn_unresolved_reports_no_url_when_the_server_has_none() {
    let (captured, _guard) = capture();
    let mut check = unresolved_check();
    check.server_url = None;
    warn_unresolved(check);
    assert!(captured.text().contains("<no url>"));
}

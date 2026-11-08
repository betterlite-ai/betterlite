#[rstest]
#[case::no_trusted(None, true)]
#[case::empty_trusted(Some(""), true)]
#[case::blank_trusted(Some("   "), true)]
fn trust_corroborate_without_a_trusted_pin_is_true(
    #[case] trusted: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(
        endpoints_corroborate(Some("https://evil.test/authorize"), trusted),
        expected
    );
}

#[rstest]
#[case::equal_after_normalization(Some("https://IDP.test:443/authorize/"), true)]
#[case::different_host(Some("https://evil.test/authorize"), false)]
#[case::missing_source(None, false)]
#[case::empty_source(Some(""), false)]
fn trust_corroborate_compares_normalized_authorize_endpoints(
    #[case] source: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(
        endpoints_corroborate(source, Some("https://idp.test/authorize")),
        expected
    );
}

#[test]
fn trust_restrict_returns_none_without_metadata() {
    assert_eq!(
        restrict_to_corroborated(
            None,
            Some("https://idp.test/authorize"),
            "test-server",
            false
        ),
        None
    );
}

#[test]
fn trust_restrict_keeps_the_metadata_without_a_manual_pin() {
    let metadata = discovered_metadata();
    assert_eq!(
        restrict_to_corroborated(Some(metadata.clone()), None, "test-server", false),
        Some(metadata.clone())
    );
    assert_eq!(
        restrict_to_corroborated(Some(metadata.clone()), Some("   "), "test-server", false),
        Some(metadata)
    );
}

#[test]
fn trust_restrict_keeps_corroborated_metadata_without_a_warning() {
    let metadata = discovered_metadata();
    let (captured, _guard) = capture();
    assert_eq!(
        restrict_to_corroborated(
            Some(metadata.clone()),
            Some("https://OTHER.test:443/authorize/"),
            "test-server",
            false
        ),
        Some(metadata)
    );
    assert_eq!(captured.text(), "");
}

#[test]
fn trust_restrict_drops_only_the_token_and_registration_urls_on_a_mismatch() {
    let metadata = discovered_metadata();
    let (captured, _guard) = capture();
    let restricted = restrict_to_corroborated(
        Some(metadata),
        Some("https://idp.test/authorize"),
        "test-server",
        false,
    )
    .unwrap();
    assert_eq!(restricted.token_url, None);
    assert_eq!(restricted.registration_url, None);
    assert_eq!(
        restricted.authorization_url.as_deref(),
        Some("https://other.test/authorize")
    );
    assert_eq!(restricted.scopes, Some(vec!["read".to_owned()]));
    let text = captured.text();
    assert!(text.contains("test-server"));
    assert!(text.contains("https://other.test/authorize"));
    assert!(text.contains("https://idp.test/authorize"));
    assert!(!text.contains("dcr_bridge"));
}

#[rstest]
#[case::registration_dropped(true, true)]
#[case::no_registration_to_drop(false, true)]
#[case::not_a_bridge(true, false)]
fn trust_restrict_adds_the_dcr_bridge_sentence_only_for_a_dropped_registration(
    #[case] with_registration: bool,
    #[case] is_dcr_bridge: bool,
) {
    let metadata = if with_registration {
        discovered_metadata()
    } else {
        metadata_from_json(
            r#"{
                "authorization_url": "https://other.test/authorize",
                "token_url": "https://other.test/token"
            }"#,
        )
    };
    let (captured, _guard) = capture();
    let restricted = restrict_to_corroborated(
        Some(metadata),
        Some("https://idp.test/authorize"),
        "test-server",
        is_dcr_bridge,
    );
    assert!(restricted.is_some());
    assert_eq!(
        captured
            .text()
            .contains("stays on the short-circuit registration arm"),
        with_registration && is_dcr_bridge
    );
}

#[rstest]
#[case::pinned_url(true)]
#[case::advertised_url(false)]
fn trust_restrict_keeps_credentials_out_of_the_warning_log(#[case] bad_pinned: bool) {
    let bad_url = "https://client:s3cret@[idp.test]/authorize?api_key=abc";
    let mut metadata = discovered_metadata();
    let manual = if bad_pinned {
        bad_url
    } else {
        metadata.authorization_url = Some(bad_url.to_owned());
        "https://idp.test/authorize"
    };
    let (captured, _guard) = capture();
    let restricted = restrict_to_corroborated(Some(metadata), Some(manual), "test-server", false);
    assert!(restricted.is_some());
    let text = captured.text();
    assert!(text.contains("<unparseable url>"));
    assert!(!text.contains("s3cret"));
    assert!(!text.contains("api_key"));
}

#[test]
fn trust_restrict_keeps_metadata_without_token_or_registration_endpoints() {
    let metadata = metadata_from_json(r#"{"authorization_url": "https://other.test/authorize"}"#);
    let (captured, _guard) = capture();
    assert_eq!(
        restrict_to_corroborated(
            Some(metadata.clone()),
            Some("https://idp.test/authorize"),
            "test-server",
            false
        ),
        Some(metadata)
    );
    assert_eq!(captured.text(), "");
}

#[test]
fn trust_carry_forward_is_skipped_without_a_previous_server_or_after_a_change() {
    let mut server = oauth_server();
    carry_forward_endpoints(&mut server, None);
    assert_eq!(server.fields.authorization_url, None);

    let mut previous = oauth_server();
    previous.fields.authorization_url = Some("https://idp.test/authorize".to_owned());

    let mut changed_url = oauth_server();
    changed_url.fields.url = Some("https://other.test/mcp".to_owned());
    carry_forward_endpoints(&mut changed_url, Some(&previous));
    assert_eq!(changed_url.fields.authorization_url, None);

    let mut changed_auth = oauth_server();
    changed_auth.fields.auth_type = Some(MCPAuth::TruePassthrough);
    carry_forward_endpoints(&mut changed_auth, Some(&previous));
    assert_eq!(changed_auth.fields.authorization_url, None);
}

#[test]
fn trust_carry_forward_on_an_anchored_server_copies_only_scopes() {
    let mut previous = oauth_server();
    previous.fields.authorization_url = Some("https://idp.test/authorize".to_owned());
    previous.fields.token_url = Some("https://idp.test/token".to_owned());
    previous.fields.registration_url = Some("https://idp.test/register".to_owned());
    previous.fields.scopes = Some(vec!["read".to_owned()]);
    let mut server = oauth_server();
    server.fields.issuer_is_anchored = true;
    carry_forward_endpoints(&mut server, Some(&previous));
    assert_eq!(server.fields.scopes, Some(vec!["read".to_owned()]));
    assert_eq!(server.fields.authorization_url, None);
    assert_eq!(server.fields.token_url, None);
    assert_eq!(server.fields.registration_url, None);
    server.fields.scopes = Some(vec!["write".to_owned()]);
    carry_forward_endpoints(&mut server, Some(&previous));
    assert_eq!(server.fields.scopes, Some(vec!["write".to_owned()]));
}

#[test]
fn trust_carry_forward_copies_endpoints_when_the_new_authorize_url_is_unpinned() {
    let mut previous = oauth_server();
    previous.fields.authorization_url = Some("https://idp.test/authorize".to_owned());
    previous.fields.token_url = Some("https://idp.test/token".to_owned());
    previous.fields.registration_url = Some("https://idp.test/register".to_owned());
    previous.fields.scopes = Some(vec!["read".to_owned()]);
    let mut server = oauth_server();
    carry_forward_endpoints(&mut server, Some(&previous));
    assert_eq!(
        server.fields.authorization_url.as_deref(),
        Some("https://idp.test/authorize")
    );
    assert_eq!(
        server.fields.token_url.as_deref(),
        Some("https://idp.test/token")
    );
    assert_eq!(
        server.fields.registration_url.as_deref(),
        Some("https://idp.test/register")
    );
    assert_eq!(server.fields.scopes, Some(vec!["read".to_owned()]));
}

#[test]
fn trust_carry_forward_keeps_the_token_endpoint_only_when_the_pin_corroborates() {
    let mut previous = oauth_server();
    previous.fields.authorization_url = Some("https://idp.test/authorize".to_owned());
    previous.fields.token_url = Some("https://idp.test/token".to_owned());
    previous.fields.registration_url = Some("https://idp.test/register".to_owned());
    previous.fields.scopes = Some(vec!["read".to_owned()]);
    let mut server = oauth_server();
    server.fields.authorization_url = Some("https://other.test/authorize".to_owned());
    carry_forward_endpoints(&mut server, Some(&previous));
    assert_eq!(
        server.fields.authorization_url.as_deref(),
        Some("https://other.test/authorize")
    );
    assert_eq!(server.fields.token_url, None);
    assert_eq!(server.fields.registration_url, None);
    assert_eq!(server.fields.scopes, None);
}

#[test]
fn trust_carry_forward_leaves_values_the_new_server_already_has() {
    let mut previous = oauth_server();
    previous.fields.authorization_url = Some("https://idp.test/authorize".to_owned());
    previous.fields.token_url = Some("https://idp.test/token".to_owned());
    previous.fields.scopes = Some(vec!["read".to_owned()]);
    let mut server = oauth_server();
    server.fields.authorization_url = Some("https://IDP.test:443/authorize/".to_owned());
    server.fields.token_url = Some("https://new.test/token".to_owned());
    server.fields.scopes = Some(vec!["write".to_owned()]);
    carry_forward_endpoints(&mut server, Some(&previous));
    assert_eq!(
        server.fields.token_url.as_deref(),
        Some("https://new.test/token")
    );
    assert_eq!(server.fields.scopes, Some(vec!["write".to_owned()]));
}

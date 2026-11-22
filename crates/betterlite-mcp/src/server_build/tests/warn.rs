#[test]
fn warn_server_name_fields_logs_invalid_values_once() {
    let (captured, _guard) = capture();
    warn_on_server_name_fields("srv-1", Some("bad alias"), Some("good-name"));
    let text = captured.text();
    assert!(text.contains("MCP server 'srv-1' has invalid alias 'bad alias':"));
    assert!(text.contains("Tool name contains spaces, which may cause parsing issues"));
    assert_eq!(text.matches("has invalid alias").count(), 1);
}

#[test]
fn warn_server_name_fields_skips_empty_and_valid_values() {
    let (captured, _guard) = capture();
    warn_on_server_name_fields("srv-1", None, Some("good-name"));
    warn_on_server_name_fields("srv-1", Some(""), Some("good_name"));
    assert!(captured.text().is_empty());
}

#[test]
fn warn_shared_identifier_prefixes_logs_once_per_group() {
    let shared_a =
        server(r#"{"server_id":"srv-b","name":"test-b","transport":"http","alias":"Shared"}"#);
    let shared_b =
        server(r#"{"server_id":"srv-a","name":"test-a","transport":"http","alias":"shared"}"#);
    let unique =
        server(r#"{"server_id":"srv-c","name":"test-c","transport":"http","server_name":"Other"}"#);
    let (captured, _guard) = capture();
    warn_on_shared_identifier_prefixes(&[shared_a, shared_b, unique]);
    let text = captured.text();
    assert!(text.contains("MCP servers ['srv-a', 'srv-b'] share the identifier 'shared';"));
    assert_eq!(text.matches("share the identifier").count(), 1);
}

#[test]
fn warn_legacy_delegate_auth_needs_no_client_credentials() {
    let legacy = server(
        r#"{"server_id":"srv-1","name":"test-server","transport":"http","auth_type":"oauth2","delegate_auth_to_upstream":true}"#,
    );
    let machine = server(
        r#"{"server_id":"srv-2","name":"test-machine","transport":"http","auth_type":"oauth2","delegate_auth_to_upstream":true,"oauth2_flow":"client_credentials"}"#,
    );
    let (captured, _guard) = capture();
    let policy = PrefixPolicy::default();
    warn_legacy_delegate_auth_if_applicable(&legacy, "database", &policy);
    warn_legacy_delegate_auth_if_applicable(&machine, "database", &policy);
    let text = captured.text();
    assert!(text.contains(
        "MCP server 'srv-1' (id=srv-1, source=database) uses deprecated auth_type=oauth2 with \
         delegate_auth_to_upstream=true."
    ));
    assert_eq!(text.matches("uses deprecated auth_type=oauth2").count(), 1);
}

#[tokio::test]
async fn warn_legacy_delegate_auth_is_called_by_the_build() {
    let harness = harness();
    let (captured, _guard) = capture();
    let source = r#"{"server_id":"srv-1","transport":"http","auth_type":"oauth2","delegate_auth_to_upstream":true}"#;
    let built = harness.build(&row(source)).await.unwrap();
    assert!(built.fields.delegate_auth_to_upstream);
    assert!(captured.text().contains("source=database"));
}

#[test]
fn warn_config_id_jag_logs_the_gap() {
    let jagged = server(
        r#"{"server_id":"srv-1","name":"test-server","transport":"http","auth_type":"oauth2_id_jag"}"#,
    );
    let other = server(
        r#"{"server_id":"srv-2","name":"test-other","transport":"http","auth_type":"oauth2"}"#,
    );
    let (captured, _guard) = capture();
    let policy = PrefixPolicy::default();
    let gap = "the token has no assertion";
    warn_config_id_jag_server_outruns_sso(&jagged, Some(gap), &policy);
    warn_config_id_jag_server_outruns_sso(&jagged, None, &policy);
    warn_config_id_jag_server_outruns_sso(&other, Some(gap), &policy);
    let text = captured.text();
    assert!(text.contains(
        "MCP server 'srv-1' (id=srv-1, source=config) is declared with \
         auth_type=oauth2_id_jag, but the token has no assertion."
    ));
    assert_eq!(
        text.matches("is declared with auth_type=oauth2_id_jag")
            .count(),
        1
    );
}

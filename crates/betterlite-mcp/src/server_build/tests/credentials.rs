#[tokio::test]
async fn credentials_decrypt_the_four_secret_fields() {
    let harness = harness();
    let table = row(
        r#"{"server_id":"srv-1","transport":"http","credentials":{"auth_value":"enc:test-token","client_id":"enc:test-client-id","client_secret":"enc:test-client-secret","client_private_key":"enc:test-private-key","client_private_key_id":"test-private-key-id"}}"#,
    );
    let server = harness.build(&table).await.unwrap();
    let token = secret_text(server.fields.authentication_token.as_ref());
    assert_eq!(token.as_deref(), Some("test-token"));
    assert_eq!(server.fields.client_id.as_deref(), Some("test-client-id"));
    let secret = secret_text(server.fields.client_secret.as_ref());
    assert_eq!(secret.as_deref(), Some("test-client-secret"));
    let private_key = secret_text(server.fields.client_private_key.as_ref());
    assert_eq!(private_key.as_deref(), Some("test-private-key"));
    assert_eq!(
        server.fields.client_private_key_id.as_deref(),
        Some("test-private-key-id")
    );
    let expected = strings(&[
        "auth_value",
        "client_id",
        "client_secret",
        "client_private_key",
    ]);
    assert_eq!(harness.decryptor.keys(), expected);
}

#[tokio::test]
async fn credentials_use_stored_values_when_not_encrypted() {
    let harness = harness();
    let options = BuildOptions {
        credentials_are_encrypted: false,
        env_vars_are_encrypted: None,
    };
    let table = row(
        r#"{"server_id":"srv-1","transport":"http","credentials":{"auth_value":"enc:test-token","client_id":"enc:test-client-id","client_secret":"enc:test-client-secret"}}"#,
    );
    let server = harness.build_with(&table, options).await.unwrap();
    let token = secret_text(server.fields.authentication_token.as_ref());
    assert_eq!(token.as_deref(), Some("enc:test-token"));
    assert_eq!(
        server.fields.client_id.as_deref(),
        Some("enc:test-client-id")
    );
    let secret = secret_text(server.fields.client_secret.as_ref());
    assert_eq!(secret.as_deref(), Some("enc:test-client-secret"));
    assert!(harness.decryptor.keys().is_empty());
}

#[tokio::test]
async fn credentials_keep_the_original_when_decryption_fails() {
    let harness = harness();
    let table = row(
        r#"{"server_id":"srv-1","transport":"http","credentials":{"client_secret":"not-decryptable"}}"#,
    );
    let server = harness.build(&table).await.unwrap();
    let secret = secret_text(server.fields.client_secret.as_ref());
    assert_eq!(secret.as_deref(), Some("not-decryptable"));
    assert_eq!(harness.decryptor.keys(), strings(&["client_secret"]));
}

#[tokio::test]
async fn credentials_empty_values_become_absent() {
    let harness = harness();
    let table = row(
        r#"{"server_id":"srv-1","transport":"http","credentials":{"auth_value":"","client_id":"","client_secret":null}}"#,
    );
    let server = harness.build(&table).await.unwrap();
    assert!(server.fields.authentication_token.is_none());
    assert!(server.fields.client_id.is_none());
    assert!(server.fields.client_secret.is_none());
    assert!(harness.decryptor.keys().is_empty());
}

#[tokio::test]
async fn credentials_decrypt_aws_secrets_only() {
    let harness = harness();
    let table = row(
        r#"{"server_id":"srv-1","transport":"http","credentials":{"aws_access_key_id":"enc:AKIATEST","aws_secret_access_key":"enc:test-aws-secret","aws_session_token":"enc:test-session-token","aws_region_name":"us-east-1","aws_service_name":"bedrock","aws_role_name":"test-role","aws_session_name":"test-session"}}"#,
    );
    let server = harness.build(&table).await.unwrap();
    let access_key = secret_text(server.fields.aws_access_key_id.as_ref());
    assert_eq!(access_key.as_deref(), Some("AKIATEST"));
    let secret_key = secret_text(server.fields.aws_secret_access_key.as_ref());
    assert_eq!(secret_key.as_deref(), Some("test-aws-secret"));
    let session = secret_text(server.fields.aws_session_token.as_ref());
    assert_eq!(session.as_deref(), Some("test-session-token"));
    assert_eq!(server.fields.aws_region_name.as_deref(), Some("us-east-1"));
    assert_eq!(server.fields.aws_service_name.as_deref(), Some("bedrock"));
    assert_eq!(server.fields.aws_role_name.as_deref(), Some("test-role"));
    assert_eq!(
        server.fields.aws_session_name.as_deref(),
        Some("test-session")
    );
    let expected = strings(&[
        "aws_access_key_id",
        "aws_secret_access_key",
        "aws_session_token",
    ]);
    assert_eq!(harness.decryptor.keys(), expected);
}

#[tokio::test]
async fn credentials_decrypt_through_the_stored_cipher() {
    let cipher = test_cipher();
    let sealed = encrypt(&cipher, "test-client-secret");
    let table = row(&format!(
        r#"{{"server_id":"srv-1","transport":"http","credentials":{{"client_secret":"{sealed}","auth_value":"not-sealed"}}}}"#
    ));
    let fetcher = FakeFetcher::default();
    let settings = settings_with_startup(false);
    let stdio = FakeStdioPolicy::default();
    let slots = FakeSlots::default();
    let builder = ServerBuilder {
        decryptor: &cipher,
        env_cipher: &cipher,
        fetcher: &fetcher,
        settings: &settings,
        stdio: &stdio,
        slots: &slots,
        prefixes: PrefixPolicy::default(),
    };
    let options = BuildOptions::default();
    let server = builder.build_from_table(&table, options).await.unwrap();
    let secret = secret_text(server.fields.client_secret.as_ref());
    assert_eq!(secret.as_deref(), Some("test-client-secret"));
    let token = secret_text(server.fields.authentication_token.as_ref());
    assert_eq!(token.as_deref(), Some("not-sealed"));
}

#[tokio::test]
async fn credentials_env_vars_decrypt_only_global_scope_when_flagged() {
    let harness = harness();
    let sealed = encrypt(&harness.cipher, "test-env-value");
    let table = row(&format!(
        r#"{{"server_id":"srv-1","transport":"http","env_vars":[{{"name":"GLOBAL","value":"{sealed}","scope":"global"}},{{"name":"USER","value":"{sealed}","scope":"user"}}]}}"#
    ));
    let options = BuildOptions {
        credentials_are_encrypted: false,
        env_vars_are_encrypted: Some(true),
    };
    let server = harness.build_with(&table, options).await.unwrap();
    let entries = server.fields.env_vars.as_ref().unwrap();
    assert_eq!(
        env_value(&entries[0]),
        Some(&JsonValue::String(String::from("test-env-value")))
    );
    assert_eq!(
        env_value(&entries[1]),
        Some(&JsonValue::String(sealed.clone()))
    );
}

#[tokio::test]
async fn credentials_env_vars_flag_falls_back_to_the_credentials_flag() {
    let harness = harness();
    let sealed = encrypt(&harness.cipher, "test-env-value");
    let table = row(&format!(
        r#"{{"server_id":"srv-1","transport":"http","env_vars":[{{"name":"GLOBAL","value":"{sealed}","scope":"global"}}]}}"#
    ));
    let plain = BuildOptions {
        credentials_are_encrypted: false,
        env_vars_are_encrypted: None,
    };
    let untouched = harness.build_with(&table, plain).await.unwrap();
    let untouched_entries = untouched.fields.env_vars.as_ref().unwrap();
    assert_eq!(
        env_value(&untouched_entries[0]),
        Some(&JsonValue::String(sealed))
    );
    let encrypted = BuildOptions {
        credentials_are_encrypted: true,
        env_vars_are_encrypted: None,
    };
    let decrypted = harness.build_with(&table, encrypted).await.unwrap();
    let decrypted_entries = decrypted.fields.env_vars.as_ref().unwrap();
    let expected = JsonValue::String(String::from("test-env-value"));
    assert_eq!(env_value(&decrypted_entries[0]), Some(&expected));
}

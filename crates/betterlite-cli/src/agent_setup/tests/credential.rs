#[test]
fn credential_uses_the_given_key() {
    let ctx = context("http://proxy.test", Some("sk-context"));
    let given = secret("sk-given");
    let resolved = resolve_credential(&ctx, Some(&given)).unwrap();
    assert_eq!(resolved.token.expose_secret(), "sk-given");
}

#[test]
fn credential_falls_back_to_the_context_key() {
    let ctx = context("http://proxy.test", Some("sk-context"));
    let resolved = resolve_credential(&ctx, None).unwrap();
    assert_eq!(resolved.token.expose_secret(), "sk-context");
}

#[test]
fn credential_ignores_a_token_file_key() {
    let ctx = token_file_context("http://proxy.test", Some("sk-login"));
    let error = resolve_credential(&ctx, None).unwrap_err();
    assert!(matches!(error, AgentSetupError::MissingCredential));
}

#[test]
fn credential_keeps_a_given_key_even_from_a_token_file_context() {
    let ctx = token_file_context("http://proxy.test", Some("sk-login"));
    let given = secret("sk-long-lived");
    let resolved = resolve_credential(&ctx, Some(&given)).unwrap();
    assert_eq!(resolved.token.expose_secret(), "sk-long-lived");
}

#[test]
fn credential_missing_key_names_every_way_to_supply_one() {
    let ctx = token_file_context("http://proxy.test", Some("sk-login"));
    let error = resolve_credential(&ctx, None).unwrap_err();
    assert!(matches!(error, AgentSetupError::MissingCredential));
    let text = error.to_string();
    assert!(text.contains("--api-key"));
    assert!(text.contains("`lite --api-key`"));
    assert!(text.contains("LITELLM_PROXY_API_KEY"));
    assert!(text.contains("lite login"));
    assert!(text.contains("expires within a day"));
    assert!(text.contains("not written into agent settings"));
}

#[rstest]
#[case::empty("")]
#[case::spaces("   ")]
#[case::non_breaking_space("\u{a0}")]
#[case::newline("sk-\nsecret")]
#[case::space("sk- secret")]
#[case::delete("sk-\u{7f}secret")]
#[case::unit_separator("sk-\u{1f}secret")]
fn credential_rejects_blank_or_control_characters(#[case] value: &str) {
    let ctx = context("http://proxy.test", None);
    let key = secret(value);
    let error = resolve_credential(&ctx, Some(&key)).unwrap_err();
    assert!(matches!(error, AgentSetupError::InvalidCredential));
    assert!(error.to_string().contains("blank or contain whitespace"));
}

#[test]
fn credential_accepts_a_one_character_key() {
    let ctx = context("http://proxy.test", None);
    let given = secret("k");
    let resolved = resolve_credential(&ctx, Some(&given)).unwrap();
    assert_eq!(resolved.token.expose_secret(), "k");
}

#[test]
fn credential_accepts_printable_punctuation() {
    let ctx = context("http://proxy.test", None);
    let given = secret("sk-!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~");
    let resolved = resolve_credential(&ctx, Some(&given)).unwrap();
    assert_eq!(
        resolved.token.expose_secret(),
        "sk-!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~"
    );
}

#[test]
fn credential_rejects_a_bad_given_key_instead_of_the_context_key() {
    let ctx = context("http://proxy.test", Some("sk-context"));
    let given = secret("  ");
    let error = resolve_credential(&ctx, Some(&given)).unwrap_err();
    assert!(matches!(error, AgentSetupError::InvalidCredential));
}

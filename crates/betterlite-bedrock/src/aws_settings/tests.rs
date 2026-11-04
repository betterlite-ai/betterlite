#![cfg(test)]

mod aws_settings_cases;

#[test]
fn invalid_tags_do_not_echo_secrets() {
    let secret = "fixture-secret-never-log";
    let error = super::canonicalize_tags(Some(vec![serde_json::json!({
        "Key": secret,
        "Value": 42
    })]))
    .unwrap_err();
    assert!(!error.to_string().contains(secret));
    assert!(!format!("{error:?}").contains(secret));
}

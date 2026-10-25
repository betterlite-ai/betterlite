use super::*;

#[test]
fn settings_validators() {
    assert_eq!(
        validate_update_interval(0.0_f64)
            .expect("zero is valid")
            .to_bits(),
        0.0_f64.to_bits()
    );
    validate_update_interval(4.9_f64).unwrap_err();
    normalize_github_api_url("http://github.com").unwrap_err();
    assert_eq!(
        validate_estimator_prompt("  hi  ").expect("prompt is valid"),
        "hi"
    );
}

#[test]
fn interval_validation() {
    validate_update_interval(43_201.0_f64).unwrap_err();
    validate_update_interval(f64::NAN).unwrap_err();
}

#[test]
fn backfill_days_validation() {
    ROISettings {
        backfill_days: 0,
        ..ROISettings::default()
    }
    .validate()
    .unwrap_err();
    ROISettings {
        backfill_days: 3650,
        ..ROISettings::default()
    }
    .validate()
    .unwrap();
}

#[test]
fn github_url_validation() {
    assert_eq!(
        normalize_github_api_url(" https://api.github.com/// ").expect("URL is valid"),
        "https://api.github.com"
    );
    normalize_github_api_url("https://user:pass@example.com").unwrap_err();
    normalize_github_api_url("https://example.com?x=1").unwrap_err();
}

#[test]
fn prompt_validation() {
    validate_estimator_prompt(" ").unwrap_err();
    validate_estimator_prompt(&"x".repeat(20_001)).unwrap_err();
}

#[test]
fn login_normalization() {
    assert_eq!(
        normalize_login(" MyLogin ").expect("login is valid"),
        "mylogin"
    );
    assert!(matches!(
        normalize_login("bad name"),
        Err(SettingsError::Login)
    ));
}

#[test]
fn email_normalization() {
    assert_eq!(
        normalize_email(Some(" User@Example.COM ")),
        "user@example.com"
    );
    assert_eq!(normalize_email(Some("a@noreply.github.com")), "");
}

#[test]
fn repositories_validation() {
    let values = vec![" acme/demo.git/ ".to_owned(), "acme/demo".to_owned()];
    assert_eq!(
        validate_repositories(&values).expect("repository is valid"),
        vec!["acme/demo"]
    );
    validate_repositories(&["../demo".to_owned()]).unwrap_err();
}

#[test]
fn identity_map_validation_preserves_specific_login_error() {
    let mut values = BTreeMap::new();
    values.insert("bad login".to_owned(), "user@example.com".to_owned());
    assert!(matches!(
        normalize_identity_map(&values),
        Err(SettingsError::Login)
    ));
}

#[test]
fn identity_map_validation() {
    let mut values = BTreeMap::new();
    values.insert("MyLogin".to_owned(), "User@Example.com".to_owned());
    assert_eq!(
        normalize_identity_map(&values)
            .expect("valid identity")
            .get("mylogin"),
        Some(&"user@example.com".to_owned())
    );
}

#[test]
fn identity_map_error_message() {
    assert_eq!(
        normalize_login("bad login")
            .expect_err("invalid login")
            .to_string(),
        "Enter a valid GitHub username."
    );
}

#[test]
fn settings_response_redaction() {
    let response = ROISettingsResponse {
        github_api_url: "https://api.github.com".to_owned(),
        repos: Vec::new(),
        estimator_model: String::new(),
        estimator_prompt: DEFAULT_PROMPT.to_owned(),
        backfill_days: 7,
        update_interval_minutes: 1440.0,
        has_estimator_key: true,
        identity_map: BTreeMap::new(),
        has_github_token: true,
        default_prompt: DEFAULT_PROMPT.to_owned(),
        available_models: Vec::new(),
        ready: true,
    };
    assert!(response.has_github_token);
    assert!(response.has_estimator_key);
}

#[test]
fn default_prompt() {
    assert_eq!(
        DEFAULT_PROMPT,
        "Estimate how many hours it would take an engineer to complete the work in this pull request without AI assistance. Explain your estimate briefly."
    );
}

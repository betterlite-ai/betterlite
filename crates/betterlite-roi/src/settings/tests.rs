#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::settings::{
    DEFAULT_PROMPT, ROISettings, ROISettingsResponse, ROISettingsUpdate, SettingsError,
    normalize_email, normalize_identity_map, normalize_login, validate_estimator_prompt,
    validate_repositories, validate_update_interval,
};

fn identities(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|&(login, email)| (login.to_owned(), email.to_owned()))
        .collect()
}

#[rstest]
#[case::url_runs_first(
    r#"{"github_api_url": "http://x", "repos": ["bad"]}"#,
    SettingsError::GitHubUrl
)]
#[case::repos_before_prompt(
    r#"{"repos": ["bad"], "estimator_prompt": " "}"#,
    SettingsError::Repositories
)]
#[case::prompt_before_days(
    r#"{"estimator_prompt": " ", "backfill_days": 0}"#,
    SettingsError::Prompt
)]
#[case::days_before_interval(
    r#"{"backfill_days": 0, "update_interval_minutes": 3}"#,
    SettingsError::TooSmall { field: "backfill_days", limit: 1 }
)]
#[case::interval_before_identities(
    r#"{"update_interval_minutes": 3, "identity_map": {"a": "bad"}}"#,
    SettingsError::Interval
)]
#[case::identities_last(r#"{"identity_map": {"a": "bad"}}"#, SettingsError::Identity)]
fn settings_validators(#[case] body: &str, #[case] expected: SettingsError) {
    let settings: ROISettings = serde_json::from_str(body).expect("settings parse");
    assert_eq!(settings.validate().unwrap_err(), expected);
}

#[test]
fn settings_validators_normalize_every_field() {
    let body = r#"{"github_api_url": " https://ghe.example.com/api/v3/ ", "repos": [" o/r.git ", "o/r/"],
        "estimator_prompt": "  count  ", "identity_map": {" Octo ": " Octo@Example.COM "}}"#;
    let parsed: ROISettings = serde_json::from_str(body).expect("settings parse");
    let settings = parsed.validate().expect("settings are valid");
    assert_eq!(settings.github_api_url, "https://ghe.example.com/api/v3");
    assert_eq!(settings.repos, ["o/r"]);
    assert_eq!(settings.estimator_prompt, "count");
    assert_eq!(
        settings.identity_map,
        identities(&[("octo", "octo@example.com")])
    );
    assert_eq!(
        (settings.backfill_days, settings.update_interval_minutes),
        (7, 1440.0_f64)
    );
}

#[rstest]
#[case::casefolds("  MyLogin ", Ok("mylogin"))]
#[case::brackets_and_dash("bot[bot]-x_1", Ok("bot[bot]-x_1"))]
#[case::sharp_s_folds_to_ascii("STRA\u{1e9e}E", Ok("strasse"))]
#[case::kelvin_sign_folds_to_ascii("\u{212a}ay", Ok("kay"))]
#[case::empty("   ", Err(SettingsError::Login))]
#[case::dot("a.b", Err(SettingsError::Login))]
#[case::inner_space("a b", Err(SettingsError::Login))]
#[case::non_ascii("jos\u{e9}", Err(SettingsError::Login))]
fn login_normalization(#[case] value: &str, #[case] expected: Result<&str, SettingsError>) {
    assert_eq!(normalize_login(value), expected.map(str::to_owned));
}

#[rstest]
#[case::casefolds(" Dev@Example.COM\u{3000}", "dev@example.com")]
#[case::folds_non_ascii("STRA\u{1e9e}E@Example.com", "strasse@example.com")]
#[case::dot_anywhere_inside_domain("a@.b.c", "a@.b.c")]
#[case::empty("", "")]
#[case::whitespace(" \t\u{3000}", "")]
#[case::no_at("dev.example.com", "")]
#[case::two_ats("a@b@c.com", "")]
#[case::no_dot("a@localhost", "")]
#[case::trailing_dot("a@b.", "")]
#[case::leading_dot_only("a@.b", "")]
#[case::inner_space("a b@c.com", "")]
#[case::noreply("1+octo@users.noreply.github.com", "")]
#[case::bare_noreply("x@noreply.github.com", "")]
#[case::noreply_lookalike("x@noreply.github.community", "x@noreply.github.community")]
fn email_normalization(#[case] value: &str, #[case] expected: &str) {
    assert_eq!(normalize_email(value), expected);
}

#[rstest]
#[case::dedupes_after_normalizing(&[" a/b.git", "a/b/", "c/d"], Ok(vec!["a/b", "c/d"]))]
#[case::removes_one_git_suffix(&["a/b.git.git"], Ok(vec!["a/b.git"]))]
#[case::slash_then_suffix(&["a/b.git/"], Ok(vec!["a/b"]))]
#[case::empty(&[], Ok(vec![]))]
#[case::dotted_names(&["my.org/re-po_1"], Ok(vec!["my.org/re-po_1"]))]
#[case::no_owner(&["repo"], Err(SettingsError::Repositories))]
#[case::three_parts(&["a/b/c"], Err(SettingsError::Repositories))]
#[case::relative_owner(&["../b"], Err(SettingsError::Repositories))]
#[case::relative_name(&["a/."], Err(SettingsError::Repositories))]
#[case::url(&["https://github.com/a/b"], Err(SettingsError::Repositories))]
#[case::invalid_after_valid(&["a/b", "a b/c"], Err(SettingsError::Repositories))]
fn repositories_validation(
    #[case] values: &[&str],
    #[case] expected: Result<Vec<&str>, SettingsError>,
) {
    let owned: Vec<String> = values.iter().map(|&value| value.to_owned()).collect();
    let expected_owned = expected.map(|list| list.into_iter().map(str::to_owned).collect());
    assert_eq!(validate_repositories(&owned), expected_owned);
}

#[rstest]
#[case::manual(0.0_f64, Ok(()))]
#[case::lowest(5.0_f64, Ok(()))]
#[case::highest(43_200.0_f64, Ok(()))]
#[case::short(4.9_f64, Err(SettingsError::Interval))]
#[case::tiny(0.001_f64, Err(SettingsError::Interval))]
#[case::negative(-1.0_f64, Err(SettingsError::TooSmall { field: "update_interval_minutes", limit: 0 }))]
#[case::over(43_200.5_f64, Err(SettingsError::TooLarge { field: "update_interval_minutes", limit: 43_200 }))]
#[case::nan(f64::NAN, Err(SettingsError::NotFinite { field: "update_interval_minutes" }))]
#[case::infinite(f64::INFINITY, Err(SettingsError::NotFinite { field: "update_interval_minutes" }))]
fn interval_validation(#[case] value: f64, #[case] expected: Result<(), SettingsError>) {
    assert_eq!(validate_update_interval(value).map(|_| ()), expected);
}

#[rstest]
#[case::update_skips_short_interval_check(r#"{"update_interval_minutes": 3}"#, Ok(()))]
#[case::update_bounds_interval(
    r#"{"update_interval_minutes": 43201}"#,
    Err(SettingsError::TooLarge { field: "update_interval_minutes", limit: 43_200 })
)]
#[case::update_bounds_days(
    r#"{"backfill_days": 3651}"#,
    Err(SettingsError::TooLarge { field: "backfill_days", limit: 3650 })
)]
fn interval_validation_on_updates(#[case] body: &str, #[case] expected: Result<(), SettingsError>) {
    let update: ROISettingsUpdate = serde_json::from_str(body).expect("update parse");
    assert_eq!(update.validate().map(|_| ()), expected);
}

#[rstest]
#[case::lowest(1_i64, Ok(()))]
#[case::highest(3650_i64, Ok(()))]
#[case::zero(0_i64, Err(SettingsError::TooSmall { field: "backfill_days", limit: 1 }))]
#[case::negative(-3_i64, Err(SettingsError::TooSmall { field: "backfill_days", limit: 1 }))]
#[case::over(3651_i64, Err(SettingsError::TooLarge { field: "backfill_days", limit: 3650 }))]
fn backfill_days_validation(#[case] days: i64, #[case] expected: Result<(), SettingsError>) {
    let settings: ROISettings =
        serde_json::from_str(&format!(r#"{{"backfill_days": {days}}}"#)).expect("settings parse");
    assert_eq!(settings.validate().map(|_| ()), expected);
}

#[rstest]
#[case::strips("\n  count hours \t", Ok("count hours".to_owned()))]
#[case::longest(&"x".repeat(20_000), Ok("x".repeat(20_000)))]
#[case::counts_characters_not_bytes(&"\u{e9}".repeat(20_000), Ok("\u{e9}".repeat(20_000)))]
#[case::too_long(&"x".repeat(20_001), Err(SettingsError::Prompt))]
#[case::empty("", Err(SettingsError::Prompt))]
#[case::blank(" \u{1f}\u{3000} ", Err(SettingsError::Prompt))]
fn prompt_validation(#[case] value: &str, #[case] expected: Result<String, SettingsError>) {
    assert_eq!(validate_estimator_prompt(value), expected);
}

#[rstest]
#[case::keeps_order(&[("Zed", "z@x.io"), ("amy", "A@x.io")], Ok(&[("zed", "z@x.io"), ("amy", "a@x.io")][..]))]
#[case::empty(&[], Ok(&[][..]))]
#[case::invalid_email(&[("amy", "amy@localhost")], Err(SettingsError::Identity))]
#[case::noreply_email(&[("amy", "1+amy@users.noreply.github.com")], Err(SettingsError::Identity))]
#[case::logins_collide(&[("Amy", "a@x.io"), ("amy", "b@x.io")], Err(SettingsError::Identity))]
#[case::bad_login_with_bad_email(&[("a b", "nope")], Err(SettingsError::Identity))]
fn identity_map_validation(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: Result<&[(&str, &str)], SettingsError>,
) {
    assert_eq!(
        normalize_identity_map(&identities(pairs)),
        expected.map(identities)
    );
}

#[rstest]
#[case::invalid_login(&[("a b", "a@x.io")])]
#[case::empty_login(&[("amy", "a@x.io"), (" ", "b@x.io")])]
fn identity_map_error_message(#[case] pairs: &[(&str, &str)]) {
    let error = normalize_identity_map(&identities(pairs)).unwrap_err();
    assert_eq!(error.to_string(), "Enter a valid GitHub username.");
}

#[test]
fn settings_response_redaction() {
    let body = r#"{"github_token": "", "estimator_key": "sk-secret", "unknown": 1}"#;
    let settings: ROISettings = serde_json::from_str(body).expect("settings parse");
    assert_eq!(settings.estimator_key.expose_secret(), "sk-secret");
    assert!(!format!("{settings:?}").contains("sk-secret"));
    let update: ROISettingsUpdate =
        serde_json::from_str(r#"{"github_token": "ghp_secret"}"#).expect("update parse");
    assert!(!format!("{update:?}").contains("ghp_secret"));
    serde_json::from_str::<ROISettingsUpdate>(r#"{"identity_map": {}}"#).unwrap_err();
    let response = ROISettingsResponse {
        github_api_url: settings.github_api_url,
        repos: settings.repos,
        estimator_model: settings.estimator_model,
        estimator_prompt: settings.estimator_prompt,
        backfill_days: settings.backfill_days,
        update_interval_minutes: settings.update_interval_minutes,
        has_estimator_key: !settings.estimator_key.expose_secret().is_empty(),
        identity_map: settings.identity_map,
        has_github_token: !settings.github_token.expose_secret().is_empty(),
        default_prompt: DEFAULT_PROMPT.to_owned(),
        available_models: vec![],
        ready: false,
    };
    let json = serde_json::to_value(&response).expect("response serializes");
    assert_eq!(json["has_estimator_key"], true);
    assert_eq!(json["has_github_token"], false);
    assert!(!json.to_string().contains("secret"));
}

#[test]
fn default_prompt() {
    assert_eq!(
        DEFAULT_PROMPT,
        "Estimate how many hours it would take an engineer to complete the work in this pull request without AI assistance. Explain your estimate briefly."
    );
    assert_eq!(ROISettings::default().estimator_prompt, DEFAULT_PROMPT);
    let settings: ROISettings = serde_json::from_str("{}").expect("settings parse");
    assert_eq!(
        settings
            .validate()
            .expect("defaults are valid")
            .estimator_prompt,
        DEFAULT_PROMPT
    );
}

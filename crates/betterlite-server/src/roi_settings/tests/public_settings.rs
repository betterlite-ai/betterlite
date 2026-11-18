#![cfg(test)]

use betterlite_roi::settings::DEFAULT_PROMPT;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::roi_settings::deployments::RouterCatalog;
use crate::roi_settings::public::public_settings;
use crate::roi_settings::support::fake_catalog;
use crate::roi_settings::support::roi_settings;

#[test]
fn public_settings_sorts_the_available_models_and_flags_ready() {
    let mut settings = roi_settings();
    settings.repos = vec!["owner/repo".to_owned()];
    settings.estimator_model = "gpt-4o".to_owned();
    let mut catalog = fake_catalog();
    catalog.names = vec![
        "gpt-4o".to_owned(),
        "claude-4".to_owned(),
        "gpt-4o".to_owned(),
    ];
    let router: &dyn RouterCatalog = &catalog;
    let response = public_settings(&settings, Some(router));
    assert_eq!(response.available_models, ["claude-4", "gpt-4o"]);
    assert!(response.ready);
}

#[rstest]
#[case::ready(vec!["owner/repo".to_owned()], "gpt-4o", vec!["gpt-4o".to_owned()], true)]
#[case::no_repos(Vec::new(), "gpt-4o", vec!["gpt-4o".to_owned()], false)]
#[case::no_estimator_model(vec!["owner/repo".to_owned()], "", vec!["gpt-4o".to_owned()], false)]
#[case::model_not_configured(vec!["owner/repo".to_owned()], "other", vec!["gpt-4o".to_owned()], false)]
#[case::no_models(vec!["owner/repo".to_owned()], "gpt-4o", Vec::new(), false)]
fn public_settings_requires_repos_and_a_known_model(
    #[case] repos: Vec<String>,
    #[case] estimator_model: &str,
    #[case] names: Vec<String>,
    #[case] ready: bool,
) {
    let mut settings = roi_settings();
    settings.repos = repos;
    settings.estimator_model = estimator_model.to_owned();
    let mut catalog = fake_catalog();
    catalog.names = names;
    let router: &dyn RouterCatalog = &catalog;
    assert_eq!(public_settings(&settings, Some(router)).ready, ready);
}

#[test]
fn public_settings_without_a_router_offers_no_models() {
    let mut settings = roi_settings();
    settings.repos = vec!["owner/repo".to_owned()];
    settings.estimator_model = "gpt-4o".to_owned();
    let response = public_settings(&settings, None);
    assert!(response.available_models.is_empty());
    assert!(!response.ready);
}

#[test]
fn public_settings_reports_the_secret_flags_without_the_values() {
    let mut settings = roi_settings();
    settings.github_token = SecretString::from("ghp_super_secret".to_owned());
    settings.estimator_key = SecretString::from("sk-estimator-secret".to_owned());
    let response = public_settings(&settings, None);
    assert!(response.has_github_token);
    assert!(response.has_estimator_key);
    let body = serde_json::to_string(&response).expect("response json");
    assert!(!body.contains("ghp_super_secret"));
    assert!(!body.contains("sk-estimator-secret"));
}

#[test]
fn public_settings_copies_every_user_visible_setting() {
    let mut identity_map = IndexMap::new();
    identity_map.insert("login".to_owned(), "user@example.com".to_owned());
    let mut settings = roi_settings();
    settings.github_api_url = "https://github.example.com".to_owned();
    settings.repos = vec!["owner/repo".to_owned()];
    settings.estimator_model = "gpt-4o".to_owned();
    settings.estimator_prompt = "Estimate the work.".to_owned();
    settings.backfill_days = 30;
    settings.update_interval_minutes = 60.0_f64;
    settings.identity_map = identity_map.clone();
    let response = public_settings(&settings, None);
    assert_eq!(response.github_api_url, settings.github_api_url);
    assert_eq!(response.repos, settings.repos);
    assert_eq!(response.estimator_model, settings.estimator_model);
    assert_eq!(response.estimator_prompt, settings.estimator_prompt);
    assert_eq!(response.backfill_days, settings.backfill_days);
    assert_eq!(
        response.update_interval_minutes.to_bits(),
        settings.update_interval_minutes.to_bits()
    );
    assert_eq!(response.identity_map, identity_map);
    assert_eq!(response.default_prompt, DEFAULT_PROMPT);
    assert!(!response.has_github_token);
    assert!(!response.has_estimator_key);
}

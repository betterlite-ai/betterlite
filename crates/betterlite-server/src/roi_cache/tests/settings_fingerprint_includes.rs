#![cfg(test)]

use betterlite_roi::settings::ROISettings;
use rstest::rstest;

use crate::roi_cache::key::settings_fingerprint;

use super::support::settings;

const LITELLM_FINGERPRINT: &str =
    "c5fda1ec6cb3ecd0d98a3a47550a48ea602b12b470f8bce55a1bf667f986a338";

#[test]
fn the_fingerprint_is_the_digest_litellm_computes_for_the_same_settings() {
    assert_eq!(
        settings_fingerprint(&settings()).unwrap(),
        LITELLM_FINGERPRINT
    );
}

fn change_url(settings: &mut ROISettings) {
    settings.github_api_url = "https://ghe.example.com/api/v3".to_owned();
}

fn change_repos(settings: &mut ROISettings) {
    settings.repos = vec!["a/b".to_owned()];
}

fn change_repo_order(settings: &mut ROISettings) {
    settings.repos = vec!["c/d".to_owned(), "a/b".to_owned()];
}

fn change_model(settings: &mut ROISettings) {
    settings.estimator_model = "other-model".to_owned();
}

fn change_prompt(settings: &mut ROISettings) {
    settings.estimator_prompt = "Estimate that.".to_owned();
}

fn change_days(settings: &mut ROISettings) {
    settings.backfill_days = 8;
}

#[rstest]
#[case::github_api_url(change_url)]
#[case::repos(change_repos)]
#[case::repo_order(change_repo_order)]
#[case::estimator_model(change_model)]
#[case::estimator_prompt(change_prompt)]
#[case::backfill_days(change_days)]
fn every_field_changes_the_fingerprint(#[case] change: fn(&mut ROISettings)) {
    let mut changed = settings();
    change(&mut changed);
    assert_ne!(
        settings_fingerprint(&changed).unwrap(),
        settings_fingerprint(&settings()).unwrap()
    );
}

#[test]
fn fields_outside_the_fingerprint_do_not_change_it() {
    let mut changed = settings();
    changed.update_interval_minutes = 60.0_f64;
    assert_eq!(
        settings_fingerprint(&changed).unwrap(),
        settings_fingerprint(&settings()).unwrap()
    );
}

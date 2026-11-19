use crate::model_reload::verdict::{
    ModelReloadVerdict, ReloadState, ServeObligation, raise_if_reload_degraded_serving,
    reload_serving_verdict,
};

use super::support::{catalog, deployment, ids, value, written_model};

#[test]
fn reload_success() {
    let published = catalog(&[deployment("gpt", "written-1", "openai/gpt-4o", true)]);
    let before = ids(&[]);
    let written = [written_model("written-1", value(r#"{"db_model":true}"#))];
    let state = ReloadState {
        before: &before,
        written: &written,
        still_desired: None,
        live_after: None,
        catalog: Some(&published),
        environment: None,
    };
    assert_eq!(
        reload_serving_verdict(&state, ServeObligation::MustServe),
        ModelReloadVerdict::default()
    );
    assert_eq!(raise_if_reload_degraded_serving(&state, "create"), Ok(()));
}

#[test]
fn missing_live_model() {
    let published = catalog(&[]);
    let before = ids(&[]);
    let written = [
        written_model("kept", value(r#"{"db_model":true}"#)),
        written_model("lost", value(r#"{"db_model":true}"#)),
    ];
    let live = ids(&["kept"]);
    let state = ReloadState {
        before: &before,
        written: &written,
        still_desired: Some(&before),
        live_after: Some(&live),
        catalog: Some(&published),
        environment: None,
    };
    let verdict = reload_serving_verdict(&state, ServeObligation::MustServe);
    assert_eq!(verdict.missing, vec!["lost".to_owned()]);
    assert!(verdict.collateral.is_empty());
    let error = raise_if_reload_degraded_serving(&state, "update").unwrap_err();
    assert_eq!(error.error_type, "internal_server_error");
    assert_eq!(error.code, "500");
    assert_eq!(error.param, None);
    assert!(
        error
            .message
            .starts_with("Model update was saved to the database")
    );
    assert!(error.message.contains("lost"));
}

#[test]
fn db_success_serving_degraded() {
    let published = catalog(&[]);
    let before = ids(&["old", "removed"]);
    let written = [written_model("kept", value(r#"{"db_model":true}"#))];
    let still_desired = ids(&["kept", "old"]);
    let live = ids(&["kept"]);
    let state = ReloadState {
        before: &before,
        written: &written,
        still_desired: Some(&still_desired),
        live_after: Some(&live),
        catalog: Some(&published),
        environment: None,
    };
    let verdict = reload_serving_verdict(&state, ServeObligation::MustServe);
    assert!(verdict.missing.is_empty());
    assert_eq!(verdict.collateral, vec!["old".to_owned()]);
    let error = raise_if_reload_degraded_serving(&state, "update").unwrap_err();
    assert_eq!(error.code, "500");
    assert!(
        error
            .message
            .contains("the reload it triggered degraded this pod's serving state.")
    );
    assert!(error.message.contains("old"));
}

#[test]
fn no_serving_on_this_pod() {
    let before = ids(&[]);
    let written = [written_model(
        "inactive",
        value(r#"{"supported_environments":["development"]}"#),
    )];
    let state = ReloadState {
        before: &before,
        written: &written,
        still_desired: None,
        live_after: None,
        catalog: None,
        environment: Some("production"),
    };
    assert_eq!(
        reload_serving_verdict(&state, ServeObligation::MustServe),
        ModelReloadVerdict::default()
    );
    assert_eq!(raise_if_reload_degraded_serving(&state, "create"), Ok(()));
}

#[test]
fn captured_live_set_beats_the_catalog() {
    let published = catalog(&[deployment("gpt", "written-1", "openai/gpt-4o", true)]);
    let before = ids(&[]);
    let written = [written_model("written-1", value(r#"{"db_model":true}"#))];
    let live = ids(&[]);
    let state = ReloadState {
        before: &before,
        written: &written,
        still_desired: None,
        live_after: Some(&live),
        catalog: Some(&published),
        environment: None,
    };
    let verdict = reload_serving_verdict(&state, ServeObligation::MustServe);
    assert_eq!(verdict.missing, vec!["written-1".to_owned()]);
}

#[test]
fn metadata_only_never_served_is_silent() {
    let published = catalog(&[]);
    let before = ids(&["other"]);
    let written = [written_model("fresh", value(r#"{"db_model":true}"#))];
    let live = ids(&["other"]);
    let state = ReloadState {
        before: &before,
        written: &written,
        still_desired: None,
        live_after: Some(&live),
        catalog: Some(&published),
        environment: None,
    };
    assert_eq!(
        reload_serving_verdict(&state, ServeObligation::MustNotDegrade),
        ModelReloadVerdict::default()
    );
}

#[test]
fn metadata_only_reports_a_served_model_that_went_away() {
    let published = catalog(&[]);
    let before = ids(&["served"]);
    let written = [written_model("served", value(r#"{"db_model":true}"#))];
    let live = ids(&[]);
    let state = ReloadState {
        before: &before,
        written: &written,
        still_desired: None,
        live_after: Some(&live),
        catalog: Some(&published),
        environment: None,
    };
    let verdict = reload_serving_verdict(&state, ServeObligation::MustNotDegrade);
    assert_eq!(verdict.missing, vec!["served".to_owned()]);
}

#[test]
fn unreadable_model_info_still_expects_liveness() {
    let published = catalog(&[]);
    let before = ids(&[]);
    let written = [written_model("unreadable", value(r#""not json""#))];
    let live = ids(&[]);
    let state = ReloadState {
        before: &before,
        written: &written,
        still_desired: None,
        live_after: Some(&live),
        catalog: Some(&published),
        environment: Some("production"),
    };
    let verdict = reload_serving_verdict(&state, ServeObligation::MustServe);
    assert_eq!(verdict.missing, vec!["unreadable".to_owned()]);
}

#[test]
fn missing_environment_still_expects_liveness() {
    let published = catalog(&[]);
    let before = ids(&[]);
    let written = [written_model(
        "gated",
        value(r#"{"supported_environments":["production"]}"#),
    )];
    let live = ids(&[]);
    let state = ReloadState {
        before: &before,
        written: &written,
        still_desired: None,
        live_after: Some(&live),
        catalog: Some(&published),
        environment: None,
    };
    let verdict = reload_serving_verdict(&state, ServeObligation::MustServe);
    assert_eq!(verdict.missing, vec!["gated".to_owned()]);
}

#[test]
fn collateral_ignores_written_and_undesired_ids() {
    let published = catalog(&[]);
    let before = ids(&["a", "b", "c"]);
    let written = [written_model("a", value(r#"{"db_model":true}"#))];
    let still_desired = ids(&["a", "b"]);
    let live = ids(&[]);
    let state = ReloadState {
        before: &before,
        written: &written,
        still_desired: Some(&still_desired),
        live_after: Some(&live),
        catalog: Some(&published),
        environment: None,
    };
    let verdict = reload_serving_verdict(&state, ServeObligation::MustNotDegrade);
    assert_eq!(verdict.collateral, vec!["b".to_owned()]);
}

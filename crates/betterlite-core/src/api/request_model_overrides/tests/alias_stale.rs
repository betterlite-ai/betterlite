use rstest::rstest;

use super::support::{
    TestCatalog, build_service, bypass_settings, catalog, deployment, fill, input, team_principal,
    text_field,
};
use crate::api::request_model_overrides::diagnostics::OverrideDiagnostic;
use crate::api::request_model_overrides::warnings::{MAX_WARNING_KEYS, WarnOnce};

const STALE_TARGET: &str = "model_name_team-1_gone";

fn missing_deployment_warning() -> OverrideDiagnostic {
    OverrideDiagnostic::StaleAliasMissingDeployment {
        model: "gpt-4o".to_owned(),
        team_id: "team-1".to_owned(),
        aliased_target: STALE_TARGET.to_owned(),
    }
}

#[test]
fn stale_alias_to_a_deleted_deployment_is_not_applied() {
    let principal = team_principal(&[("gpt-4o", STALE_TARGET)]);
    let service = build_service(
        bypass_settings(),
        catalog(TestCatalog::default()),
        Vec::new(),
    );
    let result = service.apply(&input(principal, "gpt-4o")).unwrap();
    assert_eq!(text_field(&result.data, "model"), Some("gpt-4o"));
    assert_eq!(result.diagnostics, vec![missing_deployment_warning()]);
}

#[test]
fn stale_alias_warns_once_per_sighting() {
    let service = build_service(
        bypass_settings(),
        catalog(TestCatalog::default()),
        Vec::new(),
    );
    let first = service.apply(&input(
        team_principal(&[("gpt-4o", STALE_TARGET)]),
        "gpt-4o",
    ));
    let second = service.apply(&input(
        team_principal(&[("gpt-4o", STALE_TARGET)]),
        "gpt-4o",
    ));
    assert_eq!(
        first.unwrap().diagnostics,
        vec![missing_deployment_warning()]
    );
    let quiet = second.unwrap();
    assert_eq!(quiet.diagnostics, Vec::new());
    assert_eq!(text_field(&quiet.data, "model"), Some("gpt-4o"));
}

#[test]
fn stale_alias_without_a_catalog_is_applied() {
    let principal = team_principal(&[("gpt-4o", STALE_TARGET)]);
    let service = build_service(bypass_settings(), None, Vec::new());
    let result = service.apply(&input(principal, "gpt-4o")).unwrap();
    assert_eq!(text_field(&result.data, "model"), Some(STALE_TARGET));
    assert_eq!(result.diagnostics, Vec::new());
}

#[test]
fn stale_alias_with_a_live_deployment_and_no_siblings_is_applied() {
    let catalog_view = catalog(
        TestCatalog::default().with_group(STALE_TARGET, deployment("openai", "openai/gpt-4o")),
    );
    let principal = team_principal(&[("gpt-4o", STALE_TARGET)]);
    let service = build_service(bypass_settings(), catalog_view, Vec::new());
    let result = service.apply(&input(principal, "gpt-4o")).unwrap();
    assert_eq!(text_field(&result.data, "model"), Some(STALE_TARGET));
    assert_eq!(result.diagnostics, Vec::new());
}

#[test]
fn stale_alias_over_sibling_deployments_is_applied_with_a_warning() {
    let catalog_view = catalog(
        TestCatalog::default()
            .with_group(STALE_TARGET, deployment("openai", "openai/gpt-4o"))
            .with_team_group("team-1", "gpt-4o"),
    );
    let principal = team_principal(&[("gpt-4o", STALE_TARGET)]);
    let service = build_service(bypass_settings(), catalog_view, Vec::new());
    let result = service.apply(&input(principal, "gpt-4o")).unwrap();
    assert_eq!(text_field(&result.data, "model"), Some(STALE_TARGET));
    assert_eq!(
        result.diagnostics,
        vec![OverrideDiagnostic::StaleAliasSiblingDeployment {
            model: "gpt-4o".to_owned(),
            team_id: "team-1".to_owned(),
        }]
    );
}

#[test]
fn bypass_keeps_the_requested_model_without_a_warning() {
    let catalog_view = catalog(
        TestCatalog::default()
            .with_group(STALE_TARGET, deployment("openai", "openai/gpt-4o"))
            .with_team_group("team-1", "gpt-4o"),
    );
    let principal = team_principal(&[("gpt-4o", STALE_TARGET)]);
    let service = build_service(bypass_settings(), catalog_view, Vec::new());
    let result = service.apply(&input(principal, "gpt-4o")).unwrap();
    assert_eq!(text_field(&result.data, "model"), Some("gpt-4o"));
    assert_eq!(result.diagnostics, Vec::new());
}

#[rstest]
#[case::no_team_id(None)]
#[case::not_team_scoped(Some("team-2"))]
fn alias_outside_the_team_scope_is_applied(#[case] team_id: Option<&str>) {
    let mut principal = team_principal(&[("gpt-4o", STALE_TARGET)]);
    principal.team_id = team_id.map(str::to_owned);
    let service = build_service(
        bypass_settings(),
        catalog(TestCatalog::default()),
        Vec::new(),
    );
    let result = service.apply(&input(principal, "gpt-4o")).unwrap();
    assert_eq!(text_field(&result.data, "model"), Some(STALE_TARGET));
    assert_eq!(result.diagnostics, Vec::new());
}

#[test]
fn warn_once_keeps_the_oldest_key_at_the_cap() {
    let warn_once = WarnOnce::default();
    fill(&warn_once, MAX_WARNING_KEYS);
    assert!(!warn_once.first_seen("key-0".to_owned()));
}

#[test]
fn warn_once_evicts_the_oldest_key_past_the_cap() {
    let warn_once = WarnOnce::default();
    fill(&warn_once, MAX_WARNING_KEYS + 1);
    assert!(warn_once.first_seen("key-0".to_owned()));
}

use betterlite_models::decisions::Json;

use crate::catalog_updates::catalog::{DeploymentCatalog, TeamPatterns};
use crate::deployment_credentials::error::CredentialError;
use crate::deployment_credentials::resolve::deployment_credentials;
use crate::deployment_credentials::selection::credential_deployment;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::deployment_pricing::deployment::DeploymentReadError;
use crate::wildcard_catalog::Candidate;

use super::support::{catalog_of, row, spec, text};

fn chosen(catalog: &DeploymentCatalog, model_id: &str, team_id: Option<&str>) -> Option<String> {
    credential_deployment(catalog, model_id, team_id)
        .unwrap()
        .map(|selected| selected.deployment.id().unwrap_or_default().to_owned())
}

fn owned(model_name: &str, id: &str, team: &str) -> DeploymentSpec {
    spec(
        model_name,
        id,
        &[("model", text("openai/gpt-4o"))],
        &[("team_id", text(team))],
    )
}

fn team_public(model_name: &str, id: &str, public_name: &str) -> DeploymentSpec {
    spec(
        model_name,
        id,
        &[("model", text("openai/public"))],
        &[
            ("team_id", text("team-a")),
            ("team_public_model_name", text(public_name)),
        ],
    )
}

#[test]
fn exact_id_before_name() {
    let catalog = catalog_of(vec![
        row("shared-id", "by-name", "openai/by-name"),
        row("other-name", "shared-id", "openai/by-id"),
    ]);
    assert_eq!(
        chosen(&catalog, "shared-id", None).as_deref(),
        Some("shared-id")
    );
}

#[test]
fn exact_id_authorization_left_to_caller() {
    let catalog = catalog_of(vec![owned("other-team-model", "d1", "team-a")]);
    assert_eq!(
        chosen(&catalog, "d1", Some("team-b")).as_deref(),
        Some("d1")
    );
    assert_eq!(chosen(&catalog, "other-team-model", Some("team-b")), None);
}

#[test]
fn first_shared_or_same_team_name() {
    let catalog = catalog_of(vec![
        owned("shared", "a1", "team-a"),
        row("shared", "s1", "openai/shared"),
        owned("shared", "b1", "team-b"),
    ]);
    assert_eq!(
        chosen(&catalog, "shared", Some("team-b")).as_deref(),
        Some("s1")
    );
    assert_eq!(chosen(&catalog, "shared", None).as_deref(), Some("s1"));
}

#[test]
fn foreign_name_skipped() {
    let catalog = catalog_of(vec![
        owned("shared", "a1", "team-a"),
        owned("shared", "b1", "team-b"),
    ]);
    assert_eq!(
        chosen(&catalog, "shared", Some("team-a")).as_deref(),
        Some("a1")
    );
    assert_eq!(chosen(&catalog, "shared", Some("team-c")), None);
}

#[test]
fn caller_without_team() {
    let shared = catalog_of(vec![row("shared", "s1", "openai/shared")]);
    assert_eq!(chosen(&shared, "shared", None).as_deref(), Some("s1"));
    let only_owned = catalog_of(vec![owned("shared", "a1", "team-a")]);
    assert_eq!(chosen(&only_owned, "shared", None), None);
}

#[test]
fn first_team_public_name() {
    let catalog = catalog_of(vec![
        team_public("first-model", "p1", "pub"),
        team_public("second-model", "p2", "pub"),
    ]);
    assert_eq!(
        chosen(&catalog, "pub", Some("team-a")).as_deref(),
        Some("p1")
    );
    assert_eq!(chosen(&catalog, "pub", Some("team-b")), None);
    assert_eq!(chosen(&catalog, "pub", None), None);
}

#[test]
fn team_wildcard_before_global() {
    let catalog = catalog_of(vec![
        row("openai/*", "g1", "openai/gpt-4o"),
        spec(
            "team-model",
            "t1",
            &[("model", text("openai/gpt-4o-mini"))],
            &[
                ("team_id", text("team-a")),
                ("team_public_model_name", text("openai/*")),
            ],
        ),
    ]);
    assert_eq!(
        chosen(&catalog, "openai/gpt-4o", Some("team-a")).as_deref(),
        Some("t1")
    );
    assert_eq!(
        chosen(&catalog, "openai/gpt-4o", Some("team-b")).as_deref(),
        Some("g1")
    );
}

#[test]
fn global_wildcard_owner_filter() {
    let catalog = catalog_of(vec![
        owned("openai/*", "a1", "team-a"),
        row("openai/*", "s1", "openai/gpt-4o"),
    ]);
    assert_eq!(
        chosen(&catalog, "openai/gpt-4o", Some("team-a")).as_deref(),
        Some("a1")
    );
    assert_eq!(
        chosen(&catalog, "openai/gpt-4o", Some("team-b")).as_deref(),
        Some("s1")
    );
    assert_eq!(
        chosen(&catalog, "openai/gpt-4o", None).as_deref(),
        Some("s1")
    );
}

#[test]
fn unsupported_first_wildcard() {
    let mut patterns = TeamPatterns::default();
    patterns
        .router
        .add_pattern(
            "openai/*",
            Candidate {
                model_id: None,
                model: "openai/ghost".to_owned(),
            },
        )
        .unwrap();
    patterns
        .router
        .add_pattern(
            "openai/*",
            Candidate {
                model_id: Some("d2".to_owned()),
                model: "openai/served".to_owned(),
            },
        )
        .unwrap();
    patterns
        .router
        .record_deployment("openai/*", &row("team-model", "d2", "openai/served"));
    let mut catalog = DeploymentCatalog::default();
    catalog
        .team_pattern_routers
        .insert("team-a".to_owned(), patterns);
    assert_eq!(chosen(&catalog, "openai/ghost", Some("team-a")), None);
}

#[test]
fn typed_and_dict_candidates() {
    let catalog = catalog_of(vec![
        row("gpt-4o", "d1", "openai/gpt-4o"),
        row("openai/*", "d2", "openai/gpt-4o"),
        row("azure/*", "d3", "azure/*"),
    ]);
    let direct = credential_deployment(&catalog, "gpt-4o", None)
        .unwrap()
        .unwrap();
    assert_eq!(direct.deployment.id(), Some("d1"));
    assert_eq!(direct.model, "openai/gpt-4o");
    // a matched pattern keeps a declared model with no wildcard of its own
    let wildcard = credential_deployment(&catalog, "openai/gpt-4o-mini", None)
        .unwrap()
        .unwrap();
    assert_eq!(wildcard.deployment.id(), Some("d2"));
    assert_eq!(wildcard.model, "openai/gpt-4o");
    // a declared model with a wildcard takes the segments the request captured
    let substituted = credential_deployment(&catalog, "azure/gpt-4o", None)
        .unwrap()
        .unwrap();
    assert_eq!(substituted.deployment.id(), Some("d3"));
    assert_eq!(substituted.model, "azure/gpt-4o");
    assert_eq!(
        substituted.deployment.litellm_params.get("model"),
        Some(&text("azure/gpt-4o"))
    );
}

#[test]
fn malformed_candidate_propagates() {
    let no_model = spec("broken", "d1", &[("api_key", text("sk-key"))], &[]);
    let report = credential_deployment(&catalog_of(vec![no_model]), "broken", None).unwrap_err();
    assert!(matches!(
        report,
        CredentialError::Deployment(DeploymentReadError::MissingModel)
    ));
    let bad_provider = spec(
        "openai/*",
        "d2",
        &[
            ("model", text("openai/gpt-4o")),
            (
                "custom_llm_provider",
                Json::Number(serde_json::Number::from(5_i64)),
            ),
        ],
        &[],
    );
    let report =
        credential_deployment(&catalog_of(vec![bad_provider]), "openai/gpt-4o", None).unwrap_err();
    assert!(matches!(
        report,
        CredentialError::Deployment(DeploymentReadError::NotText {
            field: "custom_llm_provider"
        })
    ));
}

#[test]
fn stale_index_reports_the_row() {
    let mut catalog = catalog_of(vec![row("known", "d1", "openai/gpt-4o")]);
    catalog.id_index.insert("ghost".to_owned(), 7);
    let report = credential_deployment(&catalog, "ghost", None).unwrap_err();
    assert!(matches!(report, CredentialError::InvalidRow { index: 7 }));
}

#[test]
fn unknown_returns_none() {
    let catalog = catalog_of(vec![row("known", "d1", "openai/gpt-4o")]);
    assert_eq!(chosen(&catalog, "unknown", None), None);
    assert_eq!(chosen(&catalog, "unknown", Some("team-a")), None);
    assert!(deployment_credentials(&catalog, "d1").unwrap().is_some());
}

#[test]
fn blocked_first_match_no_fallback() {
    let catalog = catalog_of(vec![
        spec(
            "guarded",
            "b1",
            &[("model", text("openai/a"))],
            &[("blocked", Json::Bool(true))],
        ),
        row("guarded", "b2", "openai/b"),
    ]);
    assert_eq!(chosen(&catalog, "guarded", None), None);
    let shadowing = catalog_of(vec![
        row("openai/*", "g1", "openai/gpt-4o"),
        spec(
            "team-model",
            "t1",
            &[("model", text("openai/gpt-4o"))],
            &[
                ("team_id", text("team-a")),
                ("team_public_model_name", text("openai/*")),
                ("blocked", Json::Bool(true)),
            ],
        ),
    ]);
    assert_eq!(chosen(&shadowing, "openai/gpt-4o", Some("team-a")), None);
}

#[test]
fn missing_null_false_blocked_allowed() {
    let catalog = catalog_of(vec![
        row("missing-flag", "d1", "openai/a"),
        spec(
            "null-flag",
            "d2",
            &[("model", text("openai/b"))],
            &[("blocked", Json::Null)],
        ),
        spec(
            "false-flag",
            "d3",
            &[("model", text("openai/c"))],
            &[("blocked", Json::Bool(false))],
        ),
    ]);
    for name in ["missing-flag", "null-flag", "false-flag"] {
        assert!(chosen(&catalog, name, None).is_some());
    }
}

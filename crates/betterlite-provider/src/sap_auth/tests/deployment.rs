use rstest::rstest;
use secrecy::SecretString;

use crate::sap_auth::deployment::{
    DeploymentMode, orchestration_headers, pick_orchestration_deployment,
};
use crate::sap_auth::error::SapAuthError;

use super::support::{MapConfigurations, json};

const LISTING: &str = r#"{"resources": [
    {"scenarioId": "orchestration", "configurationId": "old", "deploymentUrl": "https://old", "createdAt": "2023-01-01T00:00:00Z"},
    {"scenarioId": "other", "configurationId": "skipped", "deploymentUrl": "https://skipped", "createdAt": "2026-01-01T00:00:00Z"},
    {"scenarioId": "orchestration", "configurationId": "wrong", "deploymentUrl": "https://wrong", "createdAt": "2025-01-01T00:00:00Z"},
    {"scenarioId": "orchestration", "configurationId": "new", "deploymentUrl": "https://new", "createdAt": "2024-01-01T00:00:00Z"}
]}"#;

fn configurations() -> MapConfigurations {
    MapConfigurations::new(&[
        ("old", r#"{"executableId": "orchestration"}"#),
        ("wrong", r#"{"executableId": "other"}"#),
        ("new", r#"{"executableId": "orchestration"}"#),
    ])
}

#[rstest]
#[case::chat(DeploymentMode::Chat)]
#[case::embedding(DeploymentMode::Embedding)]
fn the_newest_valid_orchestration_deployment_is_picked(#[case] mode: DeploymentMode) {
    let url = pick_orchestration_deployment(&json(LISTING), mode, &configurations()).unwrap();
    assert_eq!(url, "https://new");
}

#[rstest]
#[case::chat(DeploymentMode::Chat)]
#[case::embedding(DeploymentMode::Embedding)]
fn a_non_orchestration_scenario_is_not_looked_up(#[case] mode: DeploymentMode) {
    let listing = json(
        r#"{"resources": [
            {"scenarioId": "other", "configurationId": "missing"},
            {"scenarioId": "orchestration", "configurationId": "a", "deploymentUrl": "https://a", "createdAt": "2024-01-01"}
        ]}"#,
    );
    let configurations = MapConfigurations::new(&[("a", r#"{"executableId": "orchestration"}"#)]);
    let url = pick_orchestration_deployment(&listing, mode, &configurations).unwrap();
    assert_eq!(url, "https://a");
}

#[test]
fn chat_ignores_a_resource_without_a_scenario_but_embedding_fails() {
    let listing = json(r#"{"resources": [{"configurationId": "a"}]}"#);
    let configurations = MapConfigurations::new(&[]);
    let chat = pick_orchestration_deployment(&listing, DeploymentMode::Chat, &configurations);
    assert!(matches!(
        chat.unwrap_err(),
        SapAuthError::NoOrchestrationDeployment
    ));
    let embedding =
        pick_orchestration_deployment(&listing, DeploymentMode::Embedding, &configurations);
    assert!(matches!(
        embedding.unwrap_err(),
        SapAuthError::DeploymentEntry
    ));
}

#[test]
fn chat_ignores_a_configuration_without_an_executable_but_embedding_fails() {
    let listing = json(
        r#"{"resources": [{"scenarioId": "orchestration", "configurationId": "a", "deploymentUrl": "https://a", "createdAt": "2024-01-01"}]}"#,
    );
    let configurations = MapConfigurations::new(&[("a", "{}")]);
    let chat = pick_orchestration_deployment(&listing, DeploymentMode::Chat, &configurations);
    assert!(matches!(
        chat.unwrap_err(),
        SapAuthError::NoOrchestrationDeployment
    ));
    let embedding =
        pick_orchestration_deployment(&listing, DeploymentMode::Embedding, &configurations);
    assert!(matches!(
        embedding.unwrap_err(),
        SapAuthError::DeploymentEntry
    ));
}

#[rstest]
#[case::chat(DeploymentMode::Chat)]
#[case::embedding(DeploymentMode::Embedding)]
fn a_matching_resource_without_a_configuration_id_is_an_error(#[case] mode: DeploymentMode) {
    let listing = json(
        r#"{"resources": [{"scenarioId": "orchestration", "deploymentUrl": "https://a", "createdAt": "2024-01-01"}]}"#,
    );
    let error =
        pick_orchestration_deployment(&listing, mode, &MapConfigurations::new(&[])).unwrap_err();
    assert!(matches!(error, SapAuthError::DeploymentEntry));
}

#[rstest]
#[case::missing_url_chat(DeploymentMode::Chat, false)]
#[case::missing_url_embedding(DeploymentMode::Embedding, false)]
#[case::missing_created_at_chat(DeploymentMode::Chat, true)]
#[case::missing_created_at_embedding(DeploymentMode::Embedding, true)]
fn a_matching_resource_without_url_or_created_at_is_an_error(
    #[case] mode: DeploymentMode,
    #[case] drop_created_at: bool,
) {
    let listing = if drop_created_at {
        json(
            r#"{"resources": [{"scenarioId": "orchestration", "configurationId": "a", "deploymentUrl": "https://a"}]}"#,
        )
    } else {
        json(
            r#"{"resources": [{"scenarioId": "orchestration", "configurationId": "a", "createdAt": "2024-01-01"}]}"#,
        )
    };
    let configurations = MapConfigurations::new(&[("a", r#"{"executableId": "orchestration"}"#)]);
    let error = pick_orchestration_deployment(&listing, mode, &configurations).unwrap_err();
    assert!(matches!(error, SapAuthError::DeploymentEntry));
}

#[test]
fn a_configuration_that_is_not_an_object_is_an_error() {
    let listing = json(
        r#"{"resources": [{"scenarioId": "orchestration", "configurationId": "a", "deploymentUrl": "https://a", "createdAt": "2024-01-01"}]}"#,
    );
    let configurations = MapConfigurations::new(&[("a", "[]")]);
    let error =
        pick_orchestration_deployment(&listing, DeploymentMode::Chat, &configurations).unwrap_err();
    assert!(matches!(error, SapAuthError::DeploymentEntry));
}

#[test]
fn a_resource_that_is_not_an_object_is_an_error() {
    let listing = json(r#"{"resources": ["text"]}"#);
    let error =
        pick_orchestration_deployment(&listing, DeploymentMode::Chat, &MapConfigurations::new(&[]))
            .unwrap_err();
    assert!(matches!(error, SapAuthError::DeploymentEntry));
}

#[test]
fn an_equal_created_at_keeps_the_first_deployment() {
    let listing = json(
        r#"{"resources": [
            {"scenarioId": "orchestration", "configurationId": "first", "deploymentUrl": "https://first", "createdAt": "2024-01-01"},
            {"scenarioId": "orchestration", "configurationId": "second", "deploymentUrl": "https://second", "createdAt": "2024-01-01"}
        ]}"#,
    );
    let configurations = MapConfigurations::new(&[
        ("first", r#"{"executableId": "orchestration"}"#),
        ("second", r#"{"executableId": "orchestration"}"#),
    ]);
    let url =
        pick_orchestration_deployment(&listing, DeploymentMode::Chat, &configurations).unwrap();
    assert_eq!(url, "https://first");
}

#[test]
fn a_configuration_lookup_failure_is_reported() {
    let listing = json(
        r#"{"resources": [{"scenarioId": "orchestration", "configurationId": "a", "deploymentUrl": "https://a", "createdAt": "2024-01-01"}]}"#,
    );
    let configurations = MapConfigurations::new(&[]).failing("a");
    let error =
        pick_orchestration_deployment(&listing, DeploymentMode::Chat, &configurations).unwrap_err();
    assert!(matches!(error, SapAuthError::ConfigurationLookup { .. }));
}

#[rstest]
#[case::missing_resources("{}")]
#[case::resources_not_a_list(r#"{"resources": {}}"#)]
#[case::listing_not_an_object("[]")]
fn a_listing_without_a_match_is_an_error(#[case] listing: &str) {
    let error = pick_orchestration_deployment(
        &json(listing),
        DeploymentMode::Chat,
        &MapConfigurations::new(&[]),
    )
    .unwrap_err();
    assert!(matches!(error, SapAuthError::NoOrchestrationDeployment));
}

#[test]
fn the_orchestration_headers_carry_the_four_values() {
    let token = SecretString::from("Bearer token".to_owned());
    let headers = orchestration_headers(&token, "rg1").unwrap();
    let authorization = headers.get("Authorization").unwrap();
    assert_eq!(authorization.to_str().unwrap(), "Bearer token");
    assert!(authorization.is_sensitive());
    assert_eq!(
        headers.get("AI-Resource-Group").unwrap().to_str().unwrap(),
        "rg1"
    );
    assert_eq!(headers.get("Content-Type").unwrap(), "application/json");
    assert_eq!(headers.get("AI-Client-Type").unwrap(), "LiteLLM");
    assert_eq!(headers.len(), 4_usize);
}

#[test]
fn a_token_that_a_header_cannot_carry_is_an_error() {
    let token = SecretString::from("Bearer bad\nvalue".to_owned());
    let error = orchestration_headers(&token, "rg1").unwrap_err();
    assert!(matches!(error, SapAuthError::InvalidHeader { .. }));
}

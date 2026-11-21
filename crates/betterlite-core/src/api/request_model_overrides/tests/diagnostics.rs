use rstest::rstest;

use crate::api::request_model_overrides::diagnostics::{DiagnosticLevel, OverrideDiagnostic};

#[rstest]
#[case::stale_missing_deployment(
    OverrideDiagnostic::StaleAliasMissingDeployment {
        model: "gpt-4o".to_owned(),
        team_id: "team-1".to_owned(),
        aliased_target: "model_name_team-1_gone".to_owned(),
    },
    DiagnosticLevel::Warning,
    "stale_alias_missing_deployment"
)]
#[case::stale_sibling_deployment(
    OverrideDiagnostic::StaleAliasSiblingDeployment {
        model: "gpt-4o".to_owned(),
        team_id: "team-1".to_owned(),
    },
    DiagnosticLevel::Warning,
    "stale_alias_sibling_deployment"
)]
#[case::model_entry_without_credentials(
    OverrideDiagnostic::ModelEntryWithoutCredentials {
        model: "gpt-4o".to_owned(),
    },
    DiagnosticLevel::Debug,
    "model_entry_without_credentials"
)]
#[case::credential_not_found(
    OverrideDiagnostic::CredentialNotFound {
        credential: "team-azure".to_owned(),
    },
    DiagnosticLevel::Warning,
    "credential_not_found"
)]
#[case::credential_applied(
    OverrideDiagnostic::CredentialApplied {
        credential: "team-azure".to_owned(),
        model: "gpt-4o".to_owned(),
    },
    DiagnosticLevel::Debug,
    "credential_applied"
)]
fn diagnostics_keep_their_litellm_level_and_tag(
    #[case] diagnostic: OverrideDiagnostic,
    #[case] level: DiagnosticLevel,
    #[case] kind: &str,
) {
    let tag = diagnostic.tag();
    assert_eq!(tag.level, level);
    assert_eq!(tag.kind, kind);
}

#[test]
fn levels_render_as_litellm_names() {
    assert_eq!(DiagnosticLevel::Warning.as_str(), "warning");
    assert_eq!(DiagnosticLevel::Debug.as_str(), "debug");
}

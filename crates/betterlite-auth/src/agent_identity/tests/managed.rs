mod classification;
mod writes;

use rstest::rstest;

use crate::agent_identity::configuration::{IdentitySource, configuration_failure};
use crate::agent_identity::error::{
    AgentIdentityError, IDENTITY_DENIED_STATUS, POLICY_UNAVAILABLE_STATUS, raise_identity_failure,
};
use crate::agent_policy::{AgentExecutionMode, AgentIdentityBinding};
use crate::identity::{AgentIdentityFailure, EntraIdentityConfig, FailureCode};

fn identity_denied(message: &str) -> AgentIdentityFailure {
    AgentIdentityFailure {
        code: FailureCode::IdentityDenied,
        message: message.to_owned(),
    }
}

fn policy_unavailable(message: &str) -> AgentIdentityFailure {
    AgentIdentityFailure {
        code: FailureCode::PolicyUnavailable,
        message: message.to_owned(),
    }
}

#[test]
fn managed_raise_failure_policy_unavailable_is_503() {
    let error = raise_identity_failure(policy_unavailable("policy unavailable"), 400);
    assert_eq!(error.status, POLICY_UNAVAILABLE_STATUS);
    assert_eq!(error.message, "policy unavailable");
}

#[test]
fn managed_raise_failure_identity_denied_defaults_to_403() {
    let error = AgentIdentityError::from(identity_denied("denied"));
    assert_eq!(error.status, IDENTITY_DENIED_STATUS);
    assert_eq!(error.message, "denied");
}

#[rstest]
#[case::bad_request(400)]
#[case::unauthorized(401)]
fn managed_raise_failure_keeps_explicit_status(#[case] status: u16) {
    let error = raise_identity_failure(identity_denied("denied"), status);
    assert_eq!(error.status, status);
}

#[test]
fn managed_configuration_failure_needs_service_principal() {
    let configuration: EntraIdentityConfig = serde_json::from_str(
        r#"{"provider":"microsoft_entra","tenant_id":"550e8400-e29b-41d4-a716-446655440000","client_id":"550e8400-e29b-41d4-a716-446655440001"}"#,
    )
    .unwrap();
    let source = IdentitySource::Configuration(&configuration);
    let failure =
        configuration_failure(Some(source), &AgentExecutionMode::Autonomous, false).unwrap();
    assert_eq!(
        failure.message,
        "Autonomous mode requires the Enterprise application service-principal object ID"
    );
    assert!(configuration_failure(Some(source), &AgentExecutionMode::Delegated, false).is_none());
}

#[test]
fn managed_configuration_failure_needs_active_binding() {
    let binding: AgentIdentityBinding = serde_json::from_str(
        r#"{"agent_id":"agent","active":false,"provider":"microsoft_entra","tenant_id":"tenant","client_id":"client","service_principal_id":"sp","issuer":"issuer","revision":"rev-1"}"#,
    )
    .unwrap();
    let source = IdentitySource::Binding(&binding);
    let failure =
        configuration_failure(Some(source), &AgentExecutionMode::Autonomous, true).unwrap();
    assert_eq!(
        failure.message,
        "Bind an identity before enabling this managed agent"
    );
    assert!(configuration_failure(None, &AgentExecutionMode::Autonomous, true).is_some());
    assert!(configuration_failure(None, &AgentExecutionMode::Autonomous, false).is_none());
}

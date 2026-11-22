#![cfg(test)]

use betterlite_auth::identity::ExecutionMode;
use betterlite_config::sections::jwt_settings::records::IssuerConfig;
use betterlite_core::agents::AgentResponse;
use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use super::{IdentityField, IdentityRequest, IdentitySettings, validate_managed_identity_request};
use crate::agent_endpoints::identity_admin::error::Detail;

const TENANT: &str = "00000000-0000-0000-0000-000000000001";
const CLIENT: &str = "00000000-0000-0000-0000-000000000002";
const ISSUER: &str = "https://login.microsoftonline.com/00000000-0000-0000-0000-000000000001/v2.0";
const OTHER_ISSUER: &str =
    "https://login.microsoftonline.com/00000000-0000-0000-0000-0000000000ff/v2.0";
const BOUND: &str = r#"{"agent_id":"agent-1","agent_name":"bound","agent_card_params":{},"execution_mode":"delegated","identity":{"agent_id":"agent-1","provider":"microsoft_entra","tenant_id":"00000000-0000-0000-0000-000000000001","client_id":"00000000-0000-0000-0000-000000000002","issuer":"https://login.microsoftonline.com/00000000-0000-0000-0000-000000000001/v2.0","revision":"rev-1"}}"#;
const PLAIN: &str = r#"{"agent_id":"plain","agent_name":"plain","agent_card_params":{}}"#;
const UNMANAGED_BINDING: &str = r#"{"agent_id":"agent-1","agent_name":"bound","agent_card_params":{},"identity":{"agent_id":"agent-1","provider":"microsoft_entra","tenant_id":"00000000-0000-0000-0000-000000000001","client_id":"00000000-0000-0000-0000-000000000002","issuer":"https://login.microsoftonline.com/00000000-0000-0000-0000-000000000001/v2.0","revision":"rev-1"}}"#;

fn entry(issuer: &str) -> IssuerConfig {
    serde_json::from_str(&format!(r#"{{"issuer":"{issuer}","audience":"api"}}"#)).unwrap()
}

fn trusting(issuer: &str) -> IdentitySettings {
    IdentitySettings {
        enable_jwt_auth: true,
        issuers: vec![entry(issuer)],
        jwt_issuer: None,
        jwt_audience: None,
        microsoft_tenant: None,
        microsoft_client_id: None,
    }
}

fn sso(tenant: &str, client: Option<&str>) -> IdentitySettings {
    IdentitySettings {
        microsoft_tenant: Some(tenant.to_owned()),
        microsoft_client_id: client.map(str::to_owned),
        ..trusting(ISSUER)
    }
}

fn existing(fields: &str) -> AgentResponse {
    serde_json::from_str(fields).unwrap()
}

fn json_value(fields: &str) -> AgentJson {
    serde_json::from_str(fields).unwrap()
}

fn configured(tenant: &str) -> AgentJson {
    json_value(&format!(
        r#"{{"provider":"microsoft_entra","tenant_id":"{tenant}","client_id":"{CLIENT}"}}"#
    ))
}

fn request(identity: IdentityField, execution_mode: Option<ExecutionMode>) -> IdentityRequest {
    IdentityRequest {
        identity,
        execution_mode,
    }
}

fn refusal(
    given: &IdentityRequest,
    stored: Option<&AgentResponse>,
    settings: &IdentitySettings,
) -> (u16, String) {
    let error = validate_managed_identity_request(given, stored, settings).unwrap_err();
    assert_eq!(error.status, 400);
    match error.detail {
        Detail::Text(message) => (error.status, message),
        Detail::Object { error: object } => panic!("a validation refuses with text, not {object}"),
    }
}

#[test]
fn an_absent_identity_without_an_agent_passes() {
    validate_managed_identity_request(
        &request(IdentityField::Absent, None),
        None,
        &trusting(ISSUER),
    )
    .unwrap();
}

#[test]
fn an_absent_identity_on_a_plain_agent_passes() {
    validate_managed_identity_request(
        &request(IdentityField::Absent, None),
        Some(&existing(PLAIN)),
        &trusting(ISSUER),
    )
    .unwrap();
}

#[test]
fn a_null_identity_passes_before_anything_is_checked() {
    validate_managed_identity_request(
        &request(IdentityField::Null, None),
        Some(&existing(BOUND)),
        &IdentitySettings::default(),
    )
    .unwrap();
}

#[rstest]
#[case::unknown_key(r#"{"provider":"microsoft_entra","tenant_id":"00000000-0000-0000-0000-000000000001","client_id":"00000000-0000-0000-0000-000000000002","scope":"read"}"#)]
#[case::wrong_provider(r#"{"provider":"google","tenant_id":"00000000-0000-0000-0000-000000000001","client_id":"00000000-0000-0000-0000-000000000002"}"#)]
#[case::tenant_not_a_uuid(r#"{"provider":"microsoft_entra","tenant_id":"tenant","client_id":"00000000-0000-0000-0000-000000000002"}"#)]
#[case::not_an_object(r#""microsoft_entra""#)]
fn an_invalid_entra_configuration_is_refused(#[case] identity: &str) {
    assert_eq!(
        refusal(
            &request(IdentityField::Value(json_value(identity)), None),
            None,
            &trusting(ISSUER),
        ),
        (400, String::from("Invalid Entra identity configuration"))
    );
}

#[test]
fn an_untrusted_issuer_is_refused() {
    assert_eq!(
        refusal(
            &request(IdentityField::Value(configured(TENANT)), None),
            None,
            &trusting(OTHER_ISSUER),
        ),
        (
            400,
            String::from(
                "Configure trusted JWT issuer and audience validation for this Entra tenant first"
            )
        )
    );
}

#[test]
fn the_issuer_is_built_from_the_normalized_tenant() {
    let normalized = configured("00000000-0000-0000-0000-00000000000A");
    validate_managed_identity_request(
        &request(IdentityField::Value(normalized), None),
        None,
        &trusting("https://login.microsoftonline.com/00000000-0000-0000-0000-00000000000a/v2.0"),
    )
    .unwrap();
}

#[rstest]
#[case::no_sso(None)]
#[case::empty_tenant(Some(""))]
#[case::another_tenant(Some("00000000-0000-0000-0000-0000000000ff"))]
fn delegated_mode_without_matching_sso_is_refused(#[case] tenant: Option<&str>) {
    let settings = match tenant {
        Some(tenant) => sso(tenant, Some("client-1")),
        None => trusting(ISSUER),
    };
    assert_eq!(
        refusal(
            &request(
                IdentityField::Value(configured(TENANT)),
                Some(ExecutionMode::Delegated),
            ),
            None,
            &settings,
        ),
        (
            400,
            String::from("Delegated agents require Microsoft SSO for the same trusted tenant")
        )
    );
}

#[rstest]
#[case::no_client(None)]
#[case::empty_client(Some(""))]
fn delegated_mode_without_a_client_is_refused(#[case] client: Option<&str>) {
    assert_eq!(
        refusal(
            &request(
                IdentityField::Value(configured(TENANT)),
                Some(ExecutionMode::Delegated),
            ),
            None,
            &sso(TENANT, client),
        ),
        (
            400,
            String::from("Delegated agents require Microsoft SSO for the same trusted tenant")
        )
    );
}

#[test]
fn both_mode_needs_the_same_sso_as_delegated() {
    assert_eq!(
        refusal(
            &request(
                IdentityField::Value(configured(TENANT)),
                Some(ExecutionMode::Both),
            ),
            None,
            &trusting(ISSUER),
        ),
        (
            400,
            String::from("Delegated agents require Microsoft SSO for the same trusted tenant")
        )
    );
}

#[rstest]
#[case::delegated(ExecutionMode::Delegated)]
#[case::both(ExecutionMode::Both)]
fn a_non_autonomous_mode_with_matching_sso_passes(#[case] mode: ExecutionMode) {
    validate_managed_identity_request(
        &request(IdentityField::Value(configured(TENANT)), Some(mode)),
        None,
        &sso(TENANT, Some("client-1")),
    )
    .unwrap();
}

#[test]
fn an_autonomous_mode_needs_no_sso() {
    validate_managed_identity_request(
        &request(
            IdentityField::Value(configured(TENANT)),
            Some(ExecutionMode::Autonomous),
        ),
        None,
        &trusting(ISSUER),
    )
    .unwrap();
}

#[test]
fn an_absent_request_identity_takes_the_stored_binding() {
    validate_managed_identity_request(
        &request(IdentityField::Absent, None),
        Some(&existing(BOUND)),
        &sso(TENANT, Some("client-1")),
    )
    .unwrap();
}

#[test]
fn a_stored_binding_still_needs_its_issuer_trusted() {
    assert_eq!(
        refusal(
            &request(IdentityField::Absent, None),
            Some(&existing(BOUND)),
            &trusting(OTHER_ISSUER),
        ),
        (
            400,
            String::from(
                "Configure trusted JWT issuer and audience validation for this Entra tenant first"
            )
        )
    );
}

#[test]
fn a_stored_delegated_mode_still_needs_sso() {
    assert_eq!(
        refusal(
            &request(IdentityField::Absent, None),
            Some(&existing(BOUND)),
            &trusting(ISSUER),
        ),
        (
            400,
            String::from("Delegated agents require Microsoft SSO for the same trusted tenant")
        )
    );
}

#[test]
fn a_stored_autonomous_mode_needs_no_sso() {
    validate_managed_identity_request(
        &request(IdentityField::Absent, None),
        Some(&existing(UNMANAGED_BINDING)),
        &trusting(ISSUER),
    )
    .unwrap();
}

#[test]
fn the_request_mode_wins_over_the_stored_one() {
    validate_managed_identity_request(
        &request(IdentityField::Absent, Some(ExecutionMode::Autonomous)),
        Some(&existing(BOUND)),
        &trusting(ISSUER),
    )
    .unwrap();
}

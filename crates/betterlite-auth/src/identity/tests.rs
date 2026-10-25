#![cfg(test)]
use super::{
    AgentIdentityBinding, AgentIdentityFailure, AgentSubject, EntraIdentityConfig,
    ManagedAgentContext, ManagedAgentIdentityStatus, MicrosoftInteractiveSubject,
    VerifiedHumanSubject,
};
use uuid::Uuid;
#[test]
fn identity_records_normalize_and_default() {
    let config: EntraIdentityConfig = serde_json::from_str(r#"{"provider":"microsoft_entra","tenant_id":"550E8400-E29B-41D4-A716-446655440000","client_id":"550e8400-e29b-41d4-a716-446655440001"}"#).unwrap();
    assert_eq!(
        config.tenant_id,
        Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
    );
    assert_eq!(config.required_scopes.0, vec!["user_impersonation"]);
    assert!(config.required_roles.is_empty());
    let service_principal = serde_json::from_str::<EntraIdentityConfig>(r#"{"provider":"microsoft_entra","tenant_id":"550e8400-e29b-41d4-a716-446655440000","client_id":"550e8400-e29b-41d4-a716-446655440001","service_principal_id":"550E8400-E29B-41D4-A716-446655440002"}"#).unwrap();
    assert_eq!(
        service_principal.service_principal_id.unwrap().to_string(),
        "550e8400-e29b-41d4-a716-446655440002"
    );
    assert_eq!(
        config.issuer(),
        "https://login.microsoftonline.com/550e8400-e29b-41d4-a716-446655440000/v2.0"
    );
    serde_json::from_str::<EntraIdentityConfig>(r#"{"provider":"microsoft_entra","tenant_id":"not-a-uuid","client_id":"550e8400-e29b-41d4-a716-446655440001"}"#).unwrap_err();
    serde_json::from_str::<EntraIdentityConfig>(r#"{"provider":"microsoft_entra","tenant_id":"550e8400-e29b-41d4-a716-446655440000","client_id":"550e8400-e29b-41d4-a716-446655440001","unknown":true}"#).unwrap_err();
    serde_json::from_str::<EntraIdentityConfig>(r#"{"provider":"google","tenant_id":"550e8400-e29b-41d4-a716-446655440000","client_id":"550e8400-e29b-41d4-a716-446655440001"}"#).unwrap_err();
}

#[test]
fn identity_records_reject_unknown_fields() {
    serde_json::from_str::<AgentIdentityBinding>(r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"550e8400-e29b-41d4-a716-446655440000","client_id":"550e8400-e29b-41d4-a716-446655440001","issuer":"i","revision":"1","unknown":true}"#).unwrap_err();
    serde_json::from_str::<AgentSubject>(
        r#"{"kind":"user","oid":"o","mode":"interactive","unknown":true}"#,
    )
    .unwrap_err();
    serde_json::from_str::<AgentIdentityFailure>(r#"{"message":"denied","unknown":true}"#)
        .unwrap_err();
    serde_json::from_str::<ManagedAgentContext>(
        r#"{"agent_id":"a","mode":"autonomous","unknown":true}"#,
    )
    .unwrap_err();
    serde_json::from_str::<VerifiedHumanSubject>(
        r#"{"issuer":"i","tenant_id":"t","oid":"o","user_id":"u","unknown":true}"#,
    )
    .unwrap_err();
    serde_json::from_str::<MicrosoftInteractiveSubject>(
        r#"{"issuer":"i","tenant_id":"t","oid":"o","unknown":true}"#,
    )
    .unwrap_err();
    serde_json::from_str::<ManagedAgentIdentityStatus>(r#"{"unknown":true}"#).unwrap_err();
}

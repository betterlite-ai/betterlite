#![cfg(test)]
use super::EntraIdentityConfig;
use uuid::Uuid;
#[test]
fn identity_records_normalize_and_default() {
    let config: EntraIdentityConfig = serde_json::from_str(r#"{"provider":"microsoft_entra","tenant_id":"550E8400-E29B-41D4-A716-446655440000","client_id":"550e8400-e29b-41d4-a716-446655440001"}"#).unwrap();
    assert_eq!(
        config.tenant_id,
        Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
    );
    assert_eq!(
        config.issuer(),
        "https://login.microsoftonline.com/550e8400-e29b-41d4-a716-446655440000/v2.0"
    );
    serde_json::from_str::<EntraIdentityConfig>(r#"{"provider":"microsoft_entra","tenant_id":"not-a-uuid","client_id":"550e8400-e29b-41d4-a716-446655440001"}"#).unwrap_err();
    serde_json::from_str::<EntraIdentityConfig>(r#"{"provider":"microsoft_entra","tenant_id":"550e8400-e29b-41d4-a716-446655440000","client_id":"550e8400-e29b-41d4-a716-446655440001","unknown":true}"#).unwrap_err();
}

#![cfg(test)]
use crate::otel::model::metadata::identity::RequestIdentity;
use crate::otel::model::utils::coercion::OtelModelValue;

#[test]
fn auth_identity_flattens_requester_metadata_and_key_fields() {
    let auth: OtelModelValue = serde_json::from_str(r#"{"team_id":"team","team_alias":"alias","api_key":"hash","user_id":"user","org_id":"org","key_alias":"key","end_user_id":"end-user","team_metadata":{"region":"west"}}"#).expect("test JSON is valid");
    let requester: OtelModelValue =
        serde_json::from_str(r#"{"requester_metadata":{"project":{"name":"demo"}}}"#)
            .expect("test JSON is valid");
    let identity = RequestIdentity::from_user_api_key_auth(&auth, Some(&requester));
    assert_eq!(identity.team_id.as_deref(), Some("team"));
    assert_eq!(identity.team_alias.as_deref(), Some("alias"));
    assert_eq!(identity.key_hash.as_deref(), Some("hash"));
    assert_eq!(identity.end_user.as_deref(), Some("end-user"));
    assert_eq!(identity.provider_model, None);
    assert_eq!(
        identity
            .metadata
            .get("requester_metadata.project.name")
            .map(String::as_str),
        Some("demo")
    );
    assert_eq!(
        identity
            .metadata
            .get("user_api_key_user_id")
            .map(String::as_str),
        Some("user")
    );
}

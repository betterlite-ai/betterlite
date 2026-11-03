#![cfg(test)]
use crate::otel::model::metadata::request_identity::from_auth::from_user_api_key_auth;
use crate::otel::model::metadata::wire::{MetadataMap, decode, payload::Auth};
use std::collections::BTreeMap;

#[test]
fn auth_identity_flattens_requester_metadata_and_key_fields() {
    let auth: Auth = serde_json::from_str(r#"{"team_id":"team","team_alias":"alias","api_key":"hash","user_id":"user","org_id":"org","key_alias":"key","end_user_id":"end-user","team_metadata":{"region":"west"}}"#).expect("test JSON is valid");
    let requester: MetadataMap =
        serde_json::from_str(r#"{"requester_metadata":{"project":{"name":"demo"}}}"#)
            .expect("test JSON is valid");
    let identity = from_user_api_key_auth(&auth, Some(&requester));
    assert_eq!(identity.team_id.as_deref(), Some("team"));
    assert_eq!(identity.team_alias.as_deref(), Some("alias"));
    assert_eq!(identity.key_hash.as_deref(), Some("hash"));
    assert_eq!(identity.end_user.as_deref(), Some("end-user"));
    assert_eq!(identity.provider_model, None);
    assert_eq!(
        identity.metadata,
        BTreeMap::from([
            (
                "requester_metadata.project.name".to_owned(),
                "demo".to_owned()
            ),
            ("user_api_key_user_id".to_owned(), "user".to_owned()),
            ("user_api_key_org_id".to_owned(), "org".to_owned()),
            ("user_api_key_alias".to_owned(), "key".to_owned()),
            ("user_api_key_end_user_id".to_owned(), "end-user".to_owned()),
        ])
    );
    assert_eq!(identity.call_id, None);
    assert_eq!(identity.request_route, None);
}

#[rstest::rstest]
#[case::team(r#"{"team_id":false}"#, "team_id")]
#[case::alias(r#"{"key_alias":17}"#, "key_alias")]
#[case::metadata(r#"{"team_metadata":[]}"#, "team_metadata")]
fn rejects_malformed_team_id(#[case] source: &str, #[case] field: &str) {
    assert!(
        decode::<Auth>(source)
            .unwrap_err()
            .to_string()
            .contains(field)
    );
}

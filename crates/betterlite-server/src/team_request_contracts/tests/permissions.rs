use super::support::parse;
use crate::team_request_contracts::permissions::{
    BulkPermissionUpdateRequest, BulkPermissionUpdateResponse, PermissionGetRequest,
    PermissionGetResponse, PermissionUpdateRequest,
};
use crate::team_request_contracts::shared::{decode_request, to_data};
use crate::team_request_contracts::values::{KeyManagementRoutes, TeamIdSearchMatch};

#[test]
fn permission_request_defaults() {
    let get: PermissionGetRequest = decode_request(&parse(r#"{"team_id": "t"}"#)).unwrap();
    assert_eq!(get.team_id, "t");
    let update: PermissionUpdateRequest = decode_request(&parse(
        r#"{"team_id": "t", "team_member_permissions": ["/key/info"]}"#,
    ))
    .unwrap();
    assert_eq!(update.team_member_permissions, vec!["/key/info".to_owned()]);
    let bulk: BulkPermissionUpdateRequest =
        decode_request(&parse(r#"{"permissions": ["/key/generate", "/key/list"]}"#)).unwrap();
    assert_eq!(bulk.permissions.len(), 2);
    assert_eq!(bulk.team_ids, None);
    assert!(!bulk.apply_to_all_teams);
}

#[test]
fn permission_request_rejects_missing_fields() {
    decode_request::<PermissionGetRequest>(&parse("{}")).unwrap_err();
    decode_request::<PermissionUpdateRequest>(&parse(r#"{"team_id": "t"}"#)).unwrap_err();
    decode_request::<PermissionUpdateRequest>(&parse(
        r#"{"team_id": "t", "team_member_permissions": null}"#,
    ))
    .unwrap_err();
    decode_request::<BulkPermissionUpdateRequest>(&parse(r#"{"team_ids": ["t"]}"#)).unwrap_err();
}

#[test]
fn permission_bulk_selection_values() {
    let explicit_null: BulkPermissionUpdateRequest = decode_request(&parse(
        r#"{"permissions": [], "team_ids": null, "apply_to_all_teams": true}"#,
    ))
    .unwrap();
    assert_eq!(explicit_null.permissions, Vec::new());
    assert_eq!(explicit_null.team_ids, None);
    assert!(explicit_null.apply_to_all_teams);
    let ids: BulkPermissionUpdateRequest = decode_request(&parse(
        r#"{"permissions": ["/key/info"], "team_ids": ["a", "b"]}"#,
    ))
    .unwrap();
    assert_eq!(ids.team_ids, Some(vec!["a".to_owned(), "b".to_owned()]));
}

#[test]
fn key_management_routes_accept_only_known() {
    let routes: KeyManagementRoutes = serde_json::from_str(r#""/key/generate""#).unwrap();
    assert_eq!(to_data(&routes).unwrap(), parse(r#""/key/generate""#));
    serde_json::from_str::<KeyManagementRoutes>(r#""key/generate""#).unwrap_err();
    serde_json::from_str::<KeyManagementRoutes>(r#""/key/nope""#).unwrap_err();
    decode_request::<BulkPermissionUpdateRequest>(&parse(r#"{"permissions": ["/nope"]}"#))
        .unwrap_err();
}

#[test]
fn permission_selection_left_to_route() {
    let both = decode_request::<BulkPermissionUpdateRequest>(&parse(
        r#"{"permissions": ["/key/info"], "team_ids": ["a"], "apply_to_all_teams": true}"#,
    ))
    .unwrap();
    assert_eq!(both.team_ids, Some(vec!["a".to_owned()]));
    assert!(both.apply_to_all_teams);
    let neither =
        decode_request::<BulkPermissionUpdateRequest>(&parse(r#"{"permissions": ["/key/info"]}"#))
            .unwrap();
    assert_eq!(neither.team_ids, None);
    assert!(!neither.apply_to_all_teams);
    let empty_ids = decode_request::<BulkPermissionUpdateRequest>(&parse(
        r#"{"permissions": ["/key/info"], "team_ids": []}"#,
    ))
    .unwrap();
    assert_eq!(empty_ids.team_ids, Some(Vec::new()));
    let all = decode_request::<BulkPermissionUpdateRequest>(&parse(
        r#"{"permissions": ["/key/info"], "apply_to_all_teams": true}"#,
    ))
    .unwrap();
    assert!(all.team_ids.is_none());
    assert!(all.apply_to_all_teams);
}

#[test]
fn permission_response_fields() {
    let defaulted: PermissionGetResponse = decode_request(&parse(
        r#"{"team_id": "t", "all_available_permissions": ["/key/info"]}"#,
    ))
    .unwrap();
    assert_eq!(defaulted.team_member_permissions, Some(Vec::new()));
    assert_eq!(
        defaulted.all_available_permissions,
        vec!["/key/info".to_owned()]
    );
    let nulled: PermissionGetResponse = decode_request(&parse(
        r#"{"team_id": "t", "team_member_permissions": null, "all_available_permissions": []}"#,
    ))
    .unwrap();
    assert_eq!(nulled.team_member_permissions, None);
    let filled: PermissionGetResponse = decode_request(&parse(
        r#"{"team_id": "t", "team_member_permissions": ["/key/info"], "all_available_permissions": []}"#,
    ))
    .unwrap();
    assert_eq!(
        filled.team_member_permissions,
        Some(vec!["/key/info".to_owned()])
    );
    assert_eq!(
        to_data(&defaulted).unwrap(),
        parse(
            r#"{"team_id": "t", "team_member_permissions": [], "all_available_permissions": ["/key/info"]}"#
        )
    );
    assert_eq!(
        to_data(&nulled).unwrap(),
        parse(
            r#"{"team_id": "t", "team_member_permissions": null, "all_available_permissions": []}"#
        )
    );
}

#[test]
fn permission_response_rejects_missing_fields() {
    decode_request::<PermissionGetResponse>(&parse(r#"{"team_id": "t"}"#)).unwrap_err();
    decode_request::<PermissionGetResponse>(&parse(r#"{"all_available_permissions": []}"#))
        .unwrap_err();
}

#[test]
fn bulk_permission_response_fields() {
    let bulk: BulkPermissionUpdateResponse =
        decode_request(&parse(r#"{"message": "ok", "teams_updated": 2}"#)).unwrap();
    assert_eq!(bulk.message, "ok");
    assert_eq!(bulk.teams_updated, 2);
    assert_eq!(bulk.permissions_appended, None);
    let appended: BulkPermissionUpdateResponse = decode_request(&parse(
        r#"{"message": "ok", "teams_updated": 0, "permissions_appended": ["/key/info"]}"#,
    ))
    .unwrap();
    assert_eq!(
        appended.permissions_appended,
        Some(vec!["/key/info".to_owned()])
    );
    assert_eq!(
        to_data(&bulk).unwrap(),
        parse(r#"{"message": "ok", "teams_updated": 2, "permissions_appended": null}"#)
    );
}

#[test]
fn bulk_permission_rejects_invalid_values() {
    decode_request::<BulkPermissionUpdateResponse>(&parse(r#"{"message": "ok"}"#)).unwrap_err();
    decode_request::<BulkPermissionUpdateResponse>(&parse(
        r#"{"message": "ok", "teams_updated": "2"}"#,
    ))
    .unwrap_err();
    decode_request::<BulkPermissionUpdateRequest>(&parse(
        r#"{"permissions": [], "apply_to_all_teams": null}"#,
    ))
    .unwrap_err();
}

#[test]
fn team_id_search_modes() {
    let exact: TeamIdSearchMatch = serde_json::from_str(r#""exact""#).unwrap();
    assert_eq!(exact, TeamIdSearchMatch::Exact);
    let prefix: TeamIdSearchMatch = serde_json::from_str(r#""prefix""#).unwrap();
    assert_eq!(prefix, TeamIdSearchMatch::Prefix);
    assert_eq!(to_data(&prefix).unwrap(), parse(r#""prefix""#));
    assert_eq!(to_data(&exact).unwrap(), parse(r#""exact""#));
    serde_json::from_str::<TeamIdSearchMatch>(r#""Exact""#).unwrap_err();
    serde_json::from_str::<TeamIdSearchMatch>(r#""fuzzy""#).unwrap_err();
    serde_json::from_str::<TeamIdSearchMatch>(r#""""#).unwrap_err();
}

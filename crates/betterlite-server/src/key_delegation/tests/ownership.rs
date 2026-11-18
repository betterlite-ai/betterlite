use axum::http::StatusCode;
use betterlite_auth::user_roles::UserRole;
use rstest::rstest;

use crate::key_delegation::caller::Caller;
use crate::key_delegation::error::{DelegationDetail, DelegationError};
use crate::key_delegation::ownership::validate_ownership_change;

use super::{admin, caller, projected, user};

#[test]
fn ownership_admin_bypass() {
    let changed = projected(r#"{"user_id": "someone"}"#);
    let removed = projected(r#"{"user_id": null}"#);
    assert_eq!(
        validate_ownership_change(Some(&changed), Some("owner"), admin()),
        Ok(())
    );
    assert_eq!(
        validate_ownership_change(Some(&removed), Some("owner"), admin()),
        Ok(())
    );
    assert_eq!(validate_ownership_change(None, None, admin()), Ok(()));
    assert_eq!(
        validate_ownership_change(
            Some(&changed),
            Some("owner"),
            caller(Some(UserRole::ProxyAdminViewer))
        ),
        Err(DelegationError::RebindUserId {
            existing: "owner".to_owned(),
            incoming: "someone".to_owned(),
        })
    );
}

#[test]
fn ownership_data_missing_or_field_omitted() {
    let omitted = projected("{}");
    let unrelated = projected(r#"{"team_id": "t"}"#);
    assert_eq!(
        validate_ownership_change(None, Some("owner"), user()),
        Ok(())
    );
    assert_eq!(
        validate_ownership_change(Some(&omitted), Some("owner"), user()),
        Ok(())
    );
    assert_eq!(
        validate_ownership_change(Some(&unrelated), None, user()),
        Ok(())
    );
}

#[rstest]
#[case::null_internal(r#"{"user_id": null}"#, user())]
#[case::null_no_role(r#"{"user_id": null}"#, caller(None))]
#[case::empty_internal(r#"{"user_id": ""}"#, user())]
#[case::empty_no_role(r#"{"user_id": ""}"#, caller(None))]
fn ownership_null_and_empty_denied(#[case] body: &str, #[case] who: Caller) {
    let request = projected(body);
    let error = validate_ownership_change(Some(&request), Some("owner"), who).unwrap_err();
    assert_eq!(error, DelegationError::RemoveUserId);
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(error.detail(), DelegationDetail::Message(error.to_string()));
    assert!(error.to_string().contains("cannot remove the user_id"));
}

#[test]
fn ownership_same_id_allowed() {
    let same = projected(r#"{"user_id": "owner"}"#);
    assert_eq!(
        validate_ownership_change(Some(&same), Some("owner"), user()),
        Ok(())
    );
    assert_eq!(
        validate_ownership_change(Some(&same), Some("owner"), caller(None)),
        Ok(())
    );
}

#[test]
fn ownership_changed_id_denied() {
    let changed = projected(r#"{"user_id": "intruder"}"#);
    let error = validate_ownership_change(Some(&changed), Some("owner"), user()).unwrap_err();
    assert_eq!(
        error,
        DelegationError::RebindUserId {
            existing: "owner".to_owned(),
            incoming: "intruder".to_owned(),
        }
    );
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(error.detail(), DelegationDetail::Message(error.to_string()));
    assert!(error.to_string().contains("user=owner"));
    assert!(error.to_string().contains("user=intruder"));
    let detached = projected(r#"{"user_id": "owner"}"#);
    assert_eq!(
        validate_ownership_change(Some(&detached), None, user()),
        Err(DelegationError::RebindUserId {
            existing: "none".to_owned(),
            incoming: "owner".to_owned(),
        })
    );
}

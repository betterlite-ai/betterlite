use axum::http::StatusCode;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use rstest::rstest;

use crate::key_delegation::error::{DelegationDetail, DelegationError};
use crate::key_delegation::permissions::check_permissions_access;
use crate::key_delegation::request::Sent;

use super::{admin, caller, json, projected, sent, user};

fn payload(entries: &[(&str, JsonData)]) -> Payload {
    Payload(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

#[rstest]
#[case::provided(projected(r#"{"permissions": {"models": ["gpt-4"]}}"#).permissions)]
#[case::explicit_null(projected(r#"{"permissions": null}"#).permissions)]
#[case::explicit_empty(projected(r#"{"permissions": {}}"#).permissions)]
#[case::omitted_truthy(sent(false, Some(payload(&[("models", json("[]"))]))))]
fn permissions_presence_and_truthiness(#[case] permissions: Sent<Payload>) {
    let error = check_permissions_access(&permissions, user()).unwrap_err();
    assert_eq!(error, DelegationError::Permissions);
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(
        error.detail(),
        DelegationDetail::Error("Only proxy admins can set `permissions`.".to_owned())
    );
    let viewer = caller(Some(UserRole::ProxyAdminViewer));
    assert_eq!(
        check_permissions_access(&permissions, viewer),
        Err(DelegationError::Permissions)
    );
    // an omitted field that stays falsey passes, and so does the absent field
    let omitted = projected("{}");
    assert_eq!(
        check_permissions_access(&omitted.permissions, user()),
        Ok(())
    );
    assert_eq!(
        check_permissions_access(&sent(false, Some(payload(&[]))), user()),
        Ok(())
    );
    assert_eq!(check_permissions_access(&sent(false, None), user()), Ok(()));
}

#[test]
fn permissions_admin_bypass() {
    let admin_caller = admin();
    let provided = projected(r#"{"permissions": {"models": ["gpt-4"]}}"#);
    let omitted_truthy = sent(false, Some(payload(&[("models", json("[]"))])));
    assert_eq!(
        check_permissions_access(&provided.permissions, admin_caller),
        Ok(())
    );
    assert_eq!(
        check_permissions_access(&sent(true, None), admin_caller),
        Ok(())
    );
    assert_eq!(
        check_permissions_access(&sent(true, Some(payload(&[]))), admin_caller),
        Ok(())
    );
    assert_eq!(
        check_permissions_access(&omitted_truthy, admin_caller),
        Ok(())
    );
    assert_eq!(
        check_permissions_access(&provided.permissions, caller(None)),
        Err(DelegationError::Permissions)
    );
}

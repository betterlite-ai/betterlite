use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use betterlite_db::budget_permission_rows::JsonData;

use super::support::{
    Harness, cadence_entry, defaults, harness_with_defaults, membership, request, user_with,
};
use crate::auth_user_lookup::error::UserLookupError;
use crate::auth_user_lookup::lookup::UserLookupRequest;

fn upsert_request<'request>(
    user_id: &'request str,
    email: Option<&'request str>,
) -> UserLookupRequest<'request> {
    let mut built = request(Some(user_id));
    built.user_email = email;
    built.user_id_upsert = true;
    built
}

#[tokio::test]
async fn scalar_defaults_override_id_and_email() {
    let harness = harness_with_defaults(Some(defaults(&[
        ("user_id", JsonData::String("override".to_owned())),
        (
            "user_email",
            JsonData::String("default@example.com".to_owned()),
        ),
        ("user_alias", JsonData::String("alias".to_owned())),
    ])));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(upsert_request("requested", Some("request@example.com")))
        .await
        .expect("the upsert")
        .expect("the created user");
    assert_eq!(user.user_id, "override");
    assert_eq!(user.user_email.as_deref(), Some("default@example.com"));
    let fields = harness.store.create_fields().unwrap();
    assert_eq!(
        fields.get("user_alias"),
        Some(&JsonData::String("alias".to_owned()))
    );

    let nulled = harness_with_defaults(Some(defaults(&[("user_email", JsonData::Null)])));
    let nulled_lookup = nulled.lookup();
    let nulled_user = nulled_lookup
        .get_user_object(upsert_request("requested", Some("request@example.com")))
        .await
        .expect("the upsert")
        .expect("the created user");
    assert_eq!(nulled_user.user_email, None);
    let nulled_fields = nulled.store.create_fields().unwrap();
    assert_eq!(nulled_fields.get("user_email"), Some(&JsonData::Null));
}

#[tokio::test]
async fn team_fields_not_user_columns() {
    let harness = harness_with_defaults(Some(defaults(&[
        (
            "teams",
            JsonData::Array(vec![JsonData::String("t1".to_owned())]),
        ),
        (
            "available_teams",
            JsonData::Array(vec![JsonData::String("t2".to_owned())]),
        ),
        ("user_role", JsonData::String("internal_user".to_owned())),
    ])));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(upsert_request("u1", None))
        .await
        .expect("the upsert")
        .expect("the created user");
    assert_eq!(user.user_id, "u1");
    let fields = harness.store.create_fields().unwrap();
    assert!(!fields.contains_key("teams"));
    assert!(!fields.contains_key("available_teams"));
    assert_eq!(
        fields.get("user_role"),
        Some(&JsonData::String("internal_user".to_owned()))
    );
}

#[tokio::test]
async fn unknown_default_field_fails_lookup() {
    let harness = harness_with_defaults(Some(defaults(&[(
        "custom_unknown",
        JsonData::Number(serde_json::Number::from(1_i64)),
    )])));
    let lookup = harness.lookup();
    let result = lookup.get_user_object(upsert_request("u1", None)).await;
    match result {
        Err(UserLookupError::UserMissing(failure)) => assert!(
            failure.to_string().contains("custom_unknown"),
            "the failure names the unknown create field: {failure}"
        ),
        other => panic!("a create field outside the user table fails the lookup: {other:?}"),
    }
    assert_eq!(harness.store.methods(), vec!["find_by_id"]);
    assert!(harness.cache.writes.lock().unwrap().is_empty());

    let relation = harness_with_defaults(Some(defaults(&[(
        "organization_memberships",
        JsonData::Array(Vec::new()),
    )])));
    let relation_lookup = relation.lookup();
    let result = relation_lookup
        .get_user_object(upsert_request("u1", None))
        .await;
    match result {
        Err(UserLookupError::UserMissing(failure)) => assert!(
            failure.to_string().contains("organization_memberships"),
            "the failure names the relation key: {failure}"
        ),
        other => panic!("a relation default fails the lookup: {other:?}"),
    }
}

#[tokio::test]
async fn password_default_reaches_create() {
    let harness = harness_with_defaults(Some(defaults(&[(
        "password",
        JsonData::String("s3cret".to_owned()),
    )])));
    let lookup = harness.lookup();
    lookup
        .get_user_object(upsert_request("u1", None))
        .await
        .expect("the upsert")
        .expect("the created user");
    let fields = harness.store.create_fields().unwrap();
    assert_eq!(
        fields.get("password"),
        Some(&JsonData::String("s3cret".to_owned()))
    );
}

#[tokio::test]
async fn absent_sso_not_added_to_create() {
    let harness = Harness::default();
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(UserLookupRequest {
            user_id: Some("u1"),
            sso_user_id: Some("sso-supplied"),
            user_email: None,
            user_id_upsert: true,
            check_db_only: None,
        })
        .await
        .expect("the upsert")
        .expect("the created user");
    assert_eq!(user.sso_user_id, None);
    let fields = harness.store.create_fields().unwrap();
    assert!(!fields.contains_key("sso_user_id"));
}

#[tokio::test]
async fn missing_reset_computed_before_create() {
    let harness = harness_with_defaults(Some(defaults(&[
        ("budget_duration", JsonData::String("1d".to_owned())),
        ("budget_reset_at", JsonData::Null),
    ])));
    let lookup = harness.lookup();
    lookup
        .get_user_object(upsert_request("u1", None))
        .await
        .expect("the upsert")
        .expect("the created user");
    assert_eq!(harness.reset.calls.lock().unwrap().clone(), vec!["1d"]);
    let fields = harness.store.create_fields().unwrap();
    let expected = serde_json::from_value::<JsonData>(
        serde_json::to_value(&harness.reset.reset).expect("the fixture timestamp serializes"),
    )
    .expect("the reset timestamp is JSON");
    assert_eq!(fields.get("budget_reset_at"), Some(&expected));
    let events = harness.events.lock().unwrap().clone();
    let reset = events.iter().position(|event| event == "reset").unwrap();
    let create = events.iter().position(|event| event == "create").unwrap();
    assert!(
        reset < create,
        "the reset is computed before the row is written"
    );

    let absent = harness_with_defaults(Some(defaults(&[(
        "budget_duration",
        JsonData::String("1d".to_owned()),
    )])));
    let absent_lookup = absent.lookup();
    absent_lookup
        .get_user_object(upsert_request("u1", None))
        .await
        .expect("the upsert")
        .expect("the created user");
    assert_eq!(absent.reset.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn explicit_reset_preserved() {
    let harness = harness_with_defaults(Some(defaults(&[
        ("budget_duration", JsonData::String("1d".to_owned())),
        (
            "budget_reset_at",
            JsonData::String("2025-05-05T00:00:00+00:00".to_owned()),
        ),
    ])));
    let lookup = harness.lookup();
    lookup
        .get_user_object(upsert_request("u1", None))
        .await
        .expect("the upsert")
        .expect("the created user");
    assert!(harness.reset.calls.lock().unwrap().is_empty());
    let fields = harness.store.create_fields().unwrap();
    assert_eq!(
        fields.get("budget_reset_at"),
        Some(&JsonData::String("2025-05-05T00:00:00+00:00".to_owned()))
    );
}

#[tokio::test]
async fn create_before_default_enrollment() {
    let harness = Harness::default();
    let lookup = harness.lookup();
    lookup
        .get_user_object(upsert_request("u1", None))
        .await
        .expect("the upsert")
        .expect("the created user");
    assert_eq!(
        harness.store.methods(),
        vec!["find_by_id", "create", "enroll"]
    );
    let events = harness.events.lock().unwrap().clone();
    let create = events.iter().position(|event| event == "create").unwrap();
    let enroll = events.iter().position(|event| event == "enroll").unwrap();
    assert!(create < enroll, "the row exists before the enrollment runs");
}

#[tokio::test]
async fn original_identity_and_admin_enrollment() {
    let harness = harness_with_defaults(Some(defaults(&[
        ("user_id", JsonData::String("override".to_owned())),
        (
            "user_email",
            JsonData::String("default@example.com".to_owned()),
        ),
    ])));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(upsert_request("requested", Some("request@example.com")))
        .await
        .expect("the upsert")
        .expect("the created user");
    assert_eq!(user.user_id, "override");
    assert_eq!(
        harness.enrollment.calls.lock().unwrap().clone(),
        vec![(
            "requested".to_owned(),
            Some("request@example.com".to_owned()),
            true,
        )]
    );
}

#[tokio::test]
async fn enrollment_port_error_keeps_created_user() {
    let harness = Harness::default();
    harness.enrollment.fail.store(true, Ordering::SeqCst);
    let lookup = harness.lookup();
    let result = lookup.get_user_object(upsert_request("u1", None)).await;
    match result {
        Err(UserLookupError::UserMissing(failure)) => assert!(
            failure
                .to_string()
                .contains("default team enrollment failed"),
            "{failure}"
        ),
        other => panic!("an enrollment failure is classified like the create path: {other:?}"),
    }
    assert_eq!(
        harness.store.methods(),
        vec!["find_by_id", "create", "enroll"]
    );
    assert!(harness.cache.writes.lock().unwrap().is_empty());
    assert_eq!(cadence_entry(&harness.cadence, "user_id:u1"), None);
}

#[tokio::test]
async fn null_memberships_filtered_in_order() {
    let harness = Harness::default();
    let mut fields = match user_with("u1", None, None) {
        JsonData::Object(fields) => fields,
        other => panic!("a user row is a mapping: {other:?}"),
    };
    fields.insert(
        "organization_memberships".to_owned(),
        JsonData::Array(vec![
            JsonData::Null,
            membership("u1", "org-a"),
            JsonData::Null,
            membership("u1", "org-b"),
            membership("u1", "org-a"),
        ]),
    );
    *harness.store.by_id.lock().unwrap() = Some(JsonData::Object(fields));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(request(Some("u1")))
        .await
        .expect("the stored user")
        .expect("the stored user is present");
    let memberships = user.organization_memberships.expect("the memberships");
    let organizations: Vec<&str> = memberships
        .iter()
        .map(|entry| entry.organization_id.as_str())
        .collect();
    assert_eq!(organizations, vec!["org-a", "org-b", "org-a"]);

    let empty = Harness::default();
    let mut empty_fields = match user_with("u2", None, None) {
        JsonData::Object(fields) => fields,
        other => panic!("a user row is a mapping: {other:?}"),
    };
    empty_fields.insert(
        "organization_memberships".to_owned(),
        JsonData::Array(Vec::new()),
    );
    *empty.store.by_id.lock().unwrap() = Some(JsonData::Object(empty_fields));
    let empty_lookup = empty.lookup();
    let empty_user = empty_lookup
        .get_user_object(request(Some("u2")))
        .await
        .expect("the stored user")
        .expect("the stored user is present");
    assert_eq!(empty_user.organization_memberships, Some(Vec::new()));
}

#[tokio::test]
async fn row_validation_before_cache() {
    let harness = Harness::default();
    *harness.store.by_id.lock().unwrap() = Some(JsonData::Object(BTreeMap::from([(
        "user_id".to_owned(),
        JsonData::Number(serde_json::Number::from(7_i64)),
    )])));
    let lookup = harness.lookup();
    let result = lookup.get_user_object(request(Some("u1"))).await;
    assert!(matches!(result, Err(UserLookupError::UserMissing(_))));
    assert!(harness.cache.writes.lock().unwrap().is_empty());
    assert_eq!(cadence_entry(&harness.cadence, "user_id:u1"), None);
    assert!(harness.store.create_fields().is_none());
}

#[tokio::test]
async fn cache_before_positive_cadence() {
    let harness = Harness::default();
    *harness.store.by_id.lock().unwrap() = Some(user_with("u1", None, None));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(request(Some("u1")))
        .await
        .expect("the stored user")
        .expect("the stored user is present");
    assert_eq!(user.user_id, "u1");
    assert_eq!(harness.cache.writes.lock().unwrap().len(), 1);
    let entry = cadence_entry(&harness.cadence, "user_id:u1").unwrap();
    assert!(entry.record.is_some());
    assert_eq!(entry.checked_at, std::time::Duration::ZERO);
}

#[tokio::test]
async fn cache_failure_keeps_committed_user() {
    let harness = Harness::default();
    harness.cache.write_fails.store(true, Ordering::SeqCst);
    let lookup = harness.lookup();
    let result = lookup.get_user_object(upsert_request("u1", None)).await;
    assert!(matches!(result, Err(UserLookupError::UserMissing(_))));
    assert!(harness.store.create_fields().is_some());
    assert_eq!(harness.enrollment.calls.lock().unwrap().len(), 1);
    assert_eq!(cadence_entry(&harness.cadence, "user_id:u1"), None);
}

#[tokio::test]
async fn upsert_after_suppressed_miss() {
    let harness = Harness::default();
    harness.clock.set(std::time::Duration::from_secs(100));
    harness.cadence.update("user_id:u1", None);
    harness.clock.set(std::time::Duration::from_secs(101));
    let lookup = harness.lookup();
    let user = lookup
        .get_user_object(upsert_request("u1", None))
        .await
        .expect("the upsert")
        .expect("the created user");
    assert_eq!(user.user_id, "u1");
    assert_eq!(harness.store.methods(), vec!["create"]);
    let entry = cadence_entry(&harness.cadence, "user_id:u1").unwrap();
    assert!(entry.record.is_some());
}

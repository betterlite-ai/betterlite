#![cfg(test)]

use super::{
    BudgetError, BudgetPermissionDecodeError, LiteLLMBudgetTable, LiteLLMObjectPermissionTable,
    OrganizationMembership, PermissionList, decode_budget_permission_rows,
};
use chrono::{DateTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

fn budget(value: serde_json::Value) -> LiteLLMBudgetTable {
    serde_json::from_value(value).expect("valid budget")
}
fn now() -> DateTime<Utc> {
    "2026-01-01T00:00:00Z".parse().expect("valid now")
}
fn membership(value: serde_json::Value) -> OrganizationMembership {
    serde_json::from_value(value).expect("valid membership")
}

#[test]
fn budget_all_twelve_defaults() {
    let row = budget(json!({}));
    assert_eq!(row.budget_id, None);
    assert_eq!(row.soft_budget, None);
    assert_eq!(row.max_budget, None);
    assert_eq!(row.max_parallel_requests, None);
    assert_eq!(row.tpm_limit, None);
    assert_eq!(row.rpm_limit, None);
    assert_eq!(row.tpd_limit, None);
    assert_eq!(row.model_max_budget, None);
    assert_eq!(row.budget_duration, None);
    assert_eq!(row.allowed_models, None);
    assert_eq!(row.temp_budget_increase, None);
    assert_eq!(row.temp_budget_expiry, None);
}

#[test]
fn full_requires_created_at() {
    assert!(
        serde_json::from_value::<super::LiteLLMBudgetTableFull>(json!({"budget": {}})).is_err()
    );
}

#[test]
fn writable_budget_ignores_server_fields() {
    let row = budget(
        json!({"budget_id":"b", "created_at":"2026-01-01T00:00:00Z", "budget_reset_at":"2026-01-02T00:00:00Z"}),
    );
    assert_eq!(row.budget_id.as_deref(), Some("b"));
}

#[test]
fn temporary_missing_amount_or_expiry() {
    let missing_amount = budget(json!({"temp_budget_expiry":"2026-01-02T00:00:00Z"}));
    let missing_expiry = budget(json!({"temp_budget_increase":2}));
    assert_eq!(
        missing_amount.active_temp_budget_increase(now()),
        Decimal::ZERO
    );
    assert_eq!(
        missing_expiry.active_temp_budget_increase(now()),
        Decimal::ZERO
    );
}

#[test]
fn temporary_before_at_after_expiry() {
    let row =
        budget(json!({"temp_budget_increase":2, "temp_budget_expiry":"2026-01-02T00:00:00Z"}));
    assert_eq!(row.active_temp_budget_increase(now()), Decimal::from(2));
    assert_eq!(
        row.active_temp_budget_increase("2026-01-02T00:00:00Z".parse().unwrap()),
        Decimal::ZERO
    );
    assert_eq!(
        row.active_temp_budget_increase("2026-01-03T00:00:00Z".parse().unwrap()),
        Decimal::ZERO
    );
}

#[test]
fn naive_expiry_is_utc() {
    let row = budget(json!({"temp_budget_increase":2, "temp_budget_expiry":"2026-01-02T00:00:00"}));
    assert_eq!(
        row.temp_budget_expiry,
        Some("2026-01-02T00:00:00Z".parse().unwrap())
    );
}

#[test]
fn offset_expiry_same_instant() {
    let row = budget(json!({"temp_budget_expiry":"2026-01-02T01:00:00+01:00"}));
    assert_eq!(
        row.temp_budget_expiry,
        Some("2026-01-02T00:00:00Z".parse().unwrap())
    );
}

#[test]
fn temporary_negative_not_clamped() {
    let row = budget(
        json!({"max_budget":10, "temp_budget_increase":-2, "temp_budget_expiry":"2026-01-02T00:00:00Z"}),
    );
    assert_eq!(
        row.effective_max_budget(now()).unwrap(),
        Some(Decimal::from(8))
    );
}

#[test]
fn unlimited_budget_ignores_increase() {
    let row =
        budget(json!({"temp_budget_increase":2, "temp_budget_expiry":"2026-01-02T00:00:00Z"}));
    assert_eq!(row.effective_max_budget(now()).unwrap(), None);
}

#[test]
fn monetary_nonfinite_and_overflow_rejected() {
    assert!(serde_json::from_str::<LiteLLMBudgetTable>(r#"{"max_budget":1e1000}"#).is_err());
    assert!(
        serde_json::from_value::<LiteLLMBudgetTable>(
            json!({"max_budget":"999999999999999999999999999999999999999999999999999999"})
        )
        .is_err()
    );
    let row = budget(
        json!({"max_budget":"79228162514264337593543950335", "temp_budget_increase":1, "temp_budget_expiry":"2026-01-02T00:00:00Z"}),
    );
    assert_eq!(row.effective_max_budget(now()), Err(BudgetError::Overflow));
}

#[test]
fn permission_all_defaults_and_required_id() {
    let row: LiteLLMObjectPermissionTable =
        serde_json::from_value(json!({"object_permission_id":"p"})).unwrap();
    assert_eq!(row.object_permission_id, "p");
    assert_eq!(row.mcp_servers, PermissionList(Some(Vec::new())));
    assert_eq!(row.mcp_toolsets, None);
    assert!(serde_json::from_value::<LiteLLMObjectPermissionTable>(json!({})).is_err());
}

#[test]
fn permission_explicit_nulls_and_fresh_lists() {
    let first: LiteLLMObjectPermissionTable =
        serde_json::from_value(json!({"object_permission_id":"p", "mcp_servers":null})).unwrap();
    let second: LiteLLMObjectPermissionTable =
        serde_json::from_value(json!({"object_permission_id":"p"})).unwrap();
    assert_eq!(first.mcp_servers, PermissionList(None));
    assert_eq!(second.mcp_servers, PermissionList(Some(Vec::new())));
}

#[test]
fn membership_required_fields_and_defaults() {
    let row = membership(
        json!({"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00", "updated_at":"2026-01-01T00:00:00Z"}),
    );
    assert_eq!(row.spend, Decimal::ZERO);
    assert_eq!(row.user_email, None);
    assert!(serde_json::from_value::<OrganizationMembership>(json!({"user_id":"u"})).is_err());
}

#[test]
fn membership_explicit_email_wins() {
    let row = membership(
        json!({"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "user_email":"direct", "user":{"user_email":"nested"}}),
    );
    assert_eq!(
        serde_json::to_value(row.user_email).unwrap(),
        json!("direct")
    );
}
#[test]
fn membership_empty_email_wins() {
    let row = membership(
        json!({"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "user_email":"", "user":{"user_email":"nested"}}),
    );
    assert_eq!(serde_json::to_value(row.user_email).unwrap(), json!(""));
}
#[test]
fn membership_dict_and_attribute_email() {
    let row = membership(
        json!({"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "user":{"user_email":"nested"}}),
    );
    assert_eq!(
        serde_json::to_value(row.user_email).unwrap(),
        json!("nested")
    );
}
#[test]
fn membership_missing_email_none() {
    let row = membership(
        json!({"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "user":{}}),
    );
    assert_eq!(row.user_email, None);
}
#[test]
fn membership_nested_nonstring_email_not_revalidated() {
    let row = membership(
        json!({"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "user":{"user_email":42}}),
    );
    assert_eq!(serde_json::to_value(row.user_email).unwrap(), json!(42));
}
#[test]
fn membership_spend_null_invalid() {
    assert!(serde_json::from_value::<OrganizationMembership>(json!({"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "spend":null})).is_err());
}
#[test]
fn strings_are_not_decoded_as_containers() {
    assert!(
        serde_json::from_value::<LiteLLMBudgetTable>(json!({"model_max_budget":"{}"})).is_err()
    );
    assert!(
        serde_json::from_value::<LiteLLMObjectPermissionTable>(
            json!({"object_permission_id":"p", "models":"[]"})
        )
        .is_err()
    );
}
#[test]
fn unknown_fields_ignored() {
    assert_eq!(budget(json!({"unknown":true})).budget_id, None);
}

#[test]
fn native_budget_rows_byte_and_depth_limits() {
    let exact = vec![b' '; 1_048_576];
    assert!(!matches!(
        decode_budget_permission_rows(&exact),
        Err(BudgetPermissionDecodeError::TooLarge { .. })
    ));
    let over = vec![b' '; 1_048_577];
    assert!(matches!(
        decode_budget_permission_rows(&over),
        Err(BudgetPermissionDecodeError::TooLarge { .. })
    ));
    let mut nested_payload = "{\"x\":".repeat(59);
    nested_payload.push('0');
    nested_payload.push_str(&"}".repeat(59));
    let nested = format!(
        "{{\"kind\":\"budget\",\"row\":{{\"model_max_budget\":{}}}}}",
        nested_payload
    );
    let nested_result = decode_budget_permission_rows(nested.as_bytes());
    assert!(nested_result.is_ok(), "{nested_result:?}");
    let mut too_deep = "[".repeat(65);
    too_deep.push('0');
    too_deep.push_str(&"]".repeat(65));
    assert!(matches!(
        decode_budget_permission_rows(too_deep.as_bytes()),
        Err(BudgetPermissionDecodeError::TooDeep)
    ));
}

#[rstest]
fn replays_the_budget_permission_rows_corpus(
    #[files("fuzz/corpus/budget_permission_rows/*")] path: PathBuf,
) {
    drop(decode_budget_permission_rows(&fs::read(path).unwrap()));
}

#[tokio::test]
async fn postgres_budget_decimal_roundtrip() {
    let fixture = betterlite_postgres_fixture::connection::PostgresFixture::provision_schema(None)
        .resolve()
        .await
        .unwrap();
    let (client, connection) =
        tokio_postgres::connect(fixture.options.url.expose_secret(), tokio_postgres::NoTls)
            .await
            .unwrap();
    tokio::spawn(async move {
        drop(connection.await);
    });
    client
        .batch_execute("CREATE TEMP TABLE budget_rows (amount DOUBLE PRECISION)")
        .await
        .unwrap();
    let value = 12.5_f64;
    client
        .execute("INSERT INTO budget_rows VALUES ($1)", &[&value])
        .await
        .unwrap();
    let read: f64 = client
        .query_one("SELECT amount FROM budget_rows", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(Decimal::try_from(read).unwrap(), Decimal::new(125, 1));
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_permission_null_vs_empty() {
    let fixture = betterlite_postgres_fixture::connection::PostgresFixture::provision_schema(None)
        .resolve()
        .await
        .unwrap();
    let (client, connection) =
        tokio_postgres::connect(fixture.options.url.expose_secret(), tokio_postgres::NoTls)
            .await
            .unwrap();
    tokio::spawn(async move {
        drop(connection.await);
    });
    client
        .batch_execute("CREATE TEMP TABLE permission_rows (models TEXT[])")
        .await
        .unwrap();
    let empty: Vec<String> = Vec::new();
    client
        .execute("INSERT INTO permission_rows VALUES (NULL), ($1)", &[&empty])
        .await
        .unwrap();
    let rows = client
        .query(
            "SELECT models FROM permission_rows ORDER BY models NULLS FIRST",
            &[],
        )
        .await
        .unwrap();
    let null_models: Option<Vec<String>> = rows.first().unwrap().get(0);
    let empty_models: Option<Vec<String>> = rows.get(1).unwrap().get(0);
    assert_eq!(null_models, None);
    assert_eq!(empty_models, Some(Vec::new()));
    fixture.close_async().await.unwrap();
}

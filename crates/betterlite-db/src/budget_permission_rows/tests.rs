#![cfg(test)]

use super::{
    BudgetError, BudgetPermissionDecodeError, LiteLLMBudgetTable, LiteLLMObjectPermissionTable,
    OrganizationMembership, PermissionList, decode_budget_permission_rows,
};
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, Utc};
use clap::Parser;
use rstest::rstest;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::{ExposeSecret as _, SecretString};
use std::fs;
use std::path::PathBuf;
use tokio::task::JoinSet;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn postgres_test_url() -> Option<SecretString> {
    let environment = Environment::parse_from(["postgres-tests"]);
    environment.url.map(SecretString::from)
}

fn budget(value: &str) -> LiteLLMBudgetTable {
    serde_json::from_str(value).expect("valid budget")
}
fn now() -> DateTime<Utc> {
    "2026-01-01T00:00:00Z".parse().expect("valid now")
}
fn membership(value: &str) -> OrganizationMembership {
    serde_json::from_str(value).expect("valid membership")
}

#[test]
fn budget_all_twelve_defaults() {
    let row = budget("{}");
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
    serde_json::from_str::<super::LiteLLMBudgetTableFull>(r#"{"budget": {}}"#).unwrap_err();
    let full: super::LiteLLMBudgetTableFull = serde_json::from_str(
        r#"{"budget_id":"b","max_budget":5,"created_at":"2026-01-01T00:00:00"}"#,
    )
    .unwrap();
    assert_eq!(full.budget.budget_id.as_deref(), Some("b"));
    assert_eq!(full.budget.max_budget, Some(Decimal::from(5_i32)));
    assert_eq!(full.budget_reset_at, None);
    assert_eq!(full.created_at.utc(), now());
}

#[test]
fn writable_budget_ignores_server_fields() {
    let row = budget(
        r#"{"budget_id":"b", "created_at":"2026-01-01T00:00:00Z", "budget_reset_at":"2026-01-02T00:00:00Z"}"#,
    );
    assert_eq!(row.budget_id.as_deref(), Some("b"));
}

#[test]
fn temporary_missing_amount_or_expiry() {
    let missing_amount = budget(r#"{"temp_budget_expiry":"2026-01-02T00:00:00Z"}"#);
    let missing_expiry = budget(r#"{"temp_budget_increase":2}"#);
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
    let row = budget(r#"{"temp_budget_increase":2, "temp_budget_expiry":"2026-01-02T00:00:00Z"}"#);
    assert_eq!(row.active_temp_budget_increase(now()), Decimal::from(2_i32));
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
    let row = budget(r#"{"temp_budget_increase":2, "temp_budget_expiry":"2026-01-02T00:00:00"}"#);
    assert_eq!(
        row.temp_budget_expiry.map(super::Timestamp::utc),
        Some("2026-01-02T00:00:00Z".parse().unwrap())
    );
}

#[test]
fn offset_expiry_same_instant() {
    let row = budget(r#"{"temp_budget_expiry":"2026-01-02T01:00:00+01:00"}"#);
    assert_eq!(
        row.temp_budget_expiry.map(super::Timestamp::utc),
        Some("2026-01-02T00:00:00Z".parse().unwrap())
    );
}

#[test]
fn temporary_negative_not_clamped() {
    let row = budget(
        r#"{"max_budget":10, "temp_budget_increase":-2, "temp_budget_expiry":"2026-01-02T00:00:00Z"}"#,
    );
    assert_eq!(
        row.effective_max_budget(now()).unwrap(),
        Some(Decimal::from(8_i32))
    );
}

#[test]
fn unlimited_budget_ignores_increase() {
    let row = budget(r#"{"temp_budget_increase":2, "temp_budget_expiry":"2026-01-02T00:00:00Z"}"#);
    assert_eq!(row.effective_max_budget(now()).unwrap(), None);
}

#[test]
fn monetary_nonfinite_and_overflow_rejected() {
    serde_json::from_str::<LiteLLMBudgetTable>(r#"{"max_budget":1e1000}"#).unwrap_err();
    serde_json::from_str::<LiteLLMBudgetTable>(
        r#"{"max_budget":"999999999999999999999999999999999999999999999999999999"}"#,
    )
    .unwrap_err();
    let row = budget(
        r#"{"max_budget":"79228162514264337593543950335", "temp_budget_increase":1, "temp_budget_expiry":"2026-01-02T00:00:00Z"}"#,
    );
    assert_eq!(row.effective_max_budget(now()), Err(BudgetError::Overflow));
}

#[test]
fn permission_all_defaults_and_required_id() {
    let row: LiteLLMObjectPermissionTable =
        serde_json::from_str(r#"{"object_permission_id":"p"}"#).unwrap();
    assert_eq!(row.object_permission_id, "p");
    assert_eq!(row.mcp_servers, PermissionList(Some(Vec::new())));
    assert_eq!(row.mcp_access_groups, PermissionList::default());
    assert_eq!(row.vector_stores, PermissionList::default());
    assert_eq!(row.agents, PermissionList::default());
    assert_eq!(row.agent_access_groups, PermissionList::default());
    assert_eq!(row.models, PermissionList::default());
    assert_eq!(row.blocked_tools, PermissionList::default());
    assert_eq!(row.search_tools, PermissionList::default());
    assert_eq!(row.mcp_toolsets, None);
    assert_eq!(row.skills, None);
    assert_eq!(row.mcp_tool_permissions, None);
    assert_eq!(row.mcp_tool_search_enabled, None);
    serde_json::from_str::<LiteLLMObjectPermissionTable>("{}").unwrap_err();
}

#[test]
fn permission_explicit_nulls_and_fresh_lists() {
    let first: LiteLLMObjectPermissionTable =
        serde_json::from_str(r#"{"object_permission_id":"p", "mcp_servers":null}"#).unwrap();
    let mut second: LiteLLMObjectPermissionTable =
        serde_json::from_str(r#"{"object_permission_id":"p"}"#).unwrap();
    assert_eq!(first.mcp_servers, PermissionList(None));
    assert_eq!(second.mcp_servers, PermissionList(Some(Vec::new())));
    second
        .mcp_servers
        .0
        .as_mut()
        .unwrap()
        .push("server".to_owned());
    let third: LiteLLMObjectPermissionTable =
        serde_json::from_str(r#"{"object_permission_id":"p"}"#).unwrap();
    assert_eq!(third.mcp_servers, PermissionList::default());
}

#[test]
fn membership_required_fields_and_defaults() {
    let row = membership(
        r#"{"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00", "updated_at":"2026-01-01T00:00:00Z"}"#,
    );
    assert_eq!(row.spend, Decimal::ZERO);
    assert_eq!(row.user_email, None);
    serde_json::from_str::<OrganizationMembership>(r#"{"user_id":"u"}"#).unwrap_err();
}

#[test]
fn membership_explicit_email_wins() {
    let row = membership(
        r#"{"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "user_email":"direct", "user":{"user_email":"nested"}}"#,
    );
    assert_eq!(
        serde_json::to_string(&row.user_email).unwrap(),
        r#""direct""#
    );
}
#[test]
fn membership_empty_email_wins() {
    let row = membership(
        r#"{"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "user_email":"", "user":{"user_email":"nested"}}"#,
    );
    assert_eq!(serde_json::to_string(&row.user_email).unwrap(), r#""""#);
}
#[test]
fn membership_dict_and_attribute_email() {
    let row = membership(
        r#"{"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "user":{"user_email":"nested"}}"#,
    );
    assert_eq!(
        serde_json::to_string(&row.user_email).unwrap(),
        r#""nested""#
    );
}
#[test]
fn membership_missing_email_none() {
    let row = membership(
        r#"{"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "user":{}}"#,
    );
    assert_eq!(row.user_email, None);
}

#[test]
fn membership_null_nested_email_is_none() {
    let row = membership(
        r#"{"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "user":{"user_email":null}}"#,
    );
    assert_eq!(row.user_email, None);
}

#[test]
fn nested_duplicate_keys_are_rejected() {
    let user = r#"{"kind":"membership","row":{"user_id":"u","organization_id":"o","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","user":{"user_email":"first","user_email":"second"}}}"#;
    let permissions = r#"{"kind":"permission","row":{"object_permission_id":"p","mcp_tool_permissions":{"tool":["first"],"tool":["second"]}}}"#;
    decode_budget_permission_rows(user.as_bytes()).unwrap_err();
    decode_budget_permission_rows(permissions.as_bytes()).unwrap_err();
}

#[test]
fn membership_nested_nonstring_email_not_revalidated() {
    let row = membership(
        r#"{"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "user":{"user_email":42}}"#,
    );
    assert_eq!(serde_json::to_string(&row.user_email).unwrap(), "42");
}
#[test]
fn membership_spend_null_invalid() {
    serde_json::from_str::<OrganizationMembership>(r#"{"user_id":"u", "organization_id":"o", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z", "spend":null}"#).unwrap_err();
}
#[test]
fn strings_are_not_decoded_as_containers() {
    serde_json::from_str::<LiteLLMBudgetTable>(r#"{"model_max_budget":"{}"}"#).unwrap_err();
    serde_json::from_str::<LiteLLMObjectPermissionTable>(
        r#"{"object_permission_id":"p", "models":"[]"}"#,
    )
    .unwrap_err();
}
#[test]
fn unknown_fields_ignored() {
    assert_eq!(budget(r#"{"unknown":true}"#).budget_id, None);
}

#[test]
fn native_budget_rows_byte_and_depth_limits() {
    let mut exact = br#"{"kind":"budget","row":{}}"#.to_vec();
    exact.resize(0x0010_0000, b' ');
    decode_budget_permission_rows(&exact).unwrap();
    let over = vec![b' '; 0x0010_0001];
    assert!(matches!(
        decode_budget_permission_rows(&over),
        Err(BudgetPermissionDecodeError::TooLarge { .. })
    ));
    let mut nested_payload = "{\"x\":".repeat(62);
    nested_payload.push('0');
    nested_payload.push_str(&"}".repeat(62));
    let nested =
        format!("{{\"kind\":\"budget\",\"row\":{{\"model_max_budget\":{nested_payload}}}}}");
    let nested_result = decode_budget_permission_rows(nested.as_bytes());
    nested_result.unwrap();
    let mut too_deep = "[".repeat(65);
    too_deep.push('0');
    too_deep.push_str(&"]".repeat(65));
    assert!(matches!(
        decode_budget_permission_rows(too_deep.as_bytes()),
        Err(BudgetPermissionDecodeError::TooDeep)
    ));
}

#[rstest]
#[case::exact_limit("exact-limit.json", 0x0010_0000)]
#[case::over_limit("over-limit.json", 0x0010_0001)]
fn corpus_byte_limit_boundaries(#[case] name: &str, #[case] size: usize) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/budget_permission_rows")
        .join(name);
    let bytes = fs::read(path).unwrap();
    assert_eq!(bytes.len(), size);
    let result = decode_budget_permission_rows(&bytes);
    if size == 0x0010_0000 {
        assert_eq!(
            result.unwrap(),
            super::BudgetPermissionRow::Budget(budget("{}"))
        );
    } else {
        assert!(matches!(
            result,
            Err(BudgetPermissionDecodeError::TooLarge { len }) if len == size
        ));
    }
}

#[rstest]
fn replays_the_budget_permission_rows_corpus(
    #[files("fuzz/corpus/budget_permission_rows/*")] path: PathBuf,
) {
    drop(decode_budget_permission_rows(&fs::read(path).unwrap()));
}

#[tokio::test]
async fn postgres_budget_decimal_roundtrip() {
    let url = postgres_test_url();
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let (client, connection) =
        tokio_postgres::connect(fixture.options.url.expose_secret(), tokio_postgres::NoTls)
            .await
            .unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    client
        .batch_execute(include_str!(
            "../../../../tools/postgres-fixture/fixtures/baseline.sql"
        ))
        .await
        .unwrap();
    let row = budget(r#"{"max_budget":12.5}"#);
    client
        .execute(r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget, created_by, updated_by) VALUES ('b', $1, 'test', 'test')"#, &[&row.max_budget.as_ref().unwrap().to_f64().unwrap()])
        .await
        .unwrap();
    let read: f64 = client
        .query_one(
            r#"SELECT max_budget FROM "LiteLLM_BudgetTable" WHERE budget_id = 'b'"#,
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(Some(Decimal::try_from(read).unwrap()), row.max_budget);
    let nonfinite: f64 = client
        .query_one("SELECT 'Infinity'::DOUBLE PRECISION", &[])
        .await
        .unwrap()
        .get(0);
    Decimal::try_from(nonfinite).unwrap_err();
    drop(client);
    tasks.join_next().await.unwrap().unwrap().unwrap();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_permission_null_vs_empty() {
    let url = postgres_test_url();
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let (client, connection) =
        tokio_postgres::connect(fixture.options.url.expose_secret(), tokio_postgres::NoTls)
            .await
            .unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    client
        .batch_execute(include_str!("permission_fixture.sql"))
        .await
        .unwrap();
    let rows = client
        .query(
            r#"SELECT row_to_json(permission)::TEXT FROM "LiteLLM_ObjectPermissionTable" permission ORDER BY object_permission_id"#,
            &[],
        )
        .await
        .unwrap();
    let decoded: Vec<LiteLLMObjectPermissionTable> = rows
        .iter()
        .map(|row| serde_json::from_str(row.get::<_, &str>(0)).unwrap())
        .collect();
    let [empty, null, omitted] = <[LiteLLMObjectPermissionTable; 3]>::try_from(decoded).unwrap();
    assert_eq!(empty.object_permission_id, "empty");
    assert_eq!(empty.models, PermissionList(Some(Vec::new())));
    assert_eq!(null.object_permission_id, "null");
    assert_eq!(null.models, PermissionList(None));
    assert_eq!(omitted.object_permission_id, "omitted");
    assert_eq!(omitted.models, PermissionList(Some(Vec::new())));
    drop(client);
    tasks.join_next().await.unwrap().unwrap().unwrap();
    fixture.close_async().await.unwrap();
}

#[test]
fn excessive_depth_fails_before_materialization() {
    let input = format!("{}0{}", "[".repeat(500), "]".repeat(500));
    assert!(matches!(
        decode_budget_permission_rows(input.as_bytes()),
        Err(BudgetPermissionDecodeError::TooDeep)
    ));
}

#[rstest]
#[case::nan("NaN")]
#[case::infinity("Infinity")]
#[case::overflow("1e100")]
fn monetary_nonfinite_strings_rejected(#[case] value: &str) {
    let input = format!(r#"{{"max_budget":"{value}"}}"#);
    serde_json::from_str::<LiteLLMBudgetTable>(&input).unwrap_err();
    let member = format!(
        r#"{{"user_id":"u","organization_id":"o","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","spend":"{value}"}}"#
    );
    serde_json::from_str::<OrganizationMembership>(&member).unwrap_err();
}

#[test]
fn direct_nonstring_email_is_rejected() {
    serde_json::from_str::<OrganizationMembership>(r#"{"user_id":"u","organization_id":"o","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","user_email":42}"#).unwrap_err();
}

#[test]
fn quoted_delimiters_do_not_count_toward_depth() {
    let input = include_bytes!("../../fuzz/corpus/budget_permission_rows/quoted-delimiters.json");
    decode_budget_permission_rows(input).unwrap();
}

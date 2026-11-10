#![cfg(test)]
mod support;
use super::contracts::RawRow;
use super::definitions::{
    DAILY_TAG_SPEND, LAST_30D_KEYS_BY_SPEND, LAST_30D_MODELS_BY_SPEND,
    LAST_30D_TOP_END_USERS_SPEND, MONTHLY_GLOBAL_SPEND, MONTHLY_GLOBAL_SPEND_PER_KEY,
    MONTHLY_USER_KEY_SPEND, RUNTIME_VERIFICATION_VIEW_SQL, RUNTIME_VIEWS, VERIFICATION_TOKEN_VIEW,
};
use super::estimate::should_create_missing_views;
use super::runtime::{create_missing_views, create_view_tolerating_race};
use chrono::{Days, NaiveDate};
use std::collections::BTreeMap;
use support::{
    Call, Database, FakeFailure, ScriptedDatabase, assert_view_columns, capture_logs,
    create_offline_views, created_view_names, execute,
};
use tokio_postgres::Client;
type VerificationRow = (String, Option<f64>, Option<f64>, Option<i64>, Option<i64>);
type KeyRow = (String, Option<String>, Option<String>, f64);
type TagRow = (String, NaiveDate, i64, f64);
type EndUserRow = (String, i64, f64);
const RUNTIME_COLUMNS: &str = "token:text,key_alias:text,key_name:text,team_id:text,project_id:text,team_spend:double precision,team_max_budget:double precision,team_model_max_budget:jsonb,team_tpm_limit:bigint,team_rpm_limit:bigint,team_tpd_limit:bigint,project_alias:text";
const OFFLINE_COLUMNS: &str = "token:text,key_alias:text,key_name:text,team_id:text,project_id:text,team_spend:double precision,team_max_budget:double precision,team_tpm_limit:bigint,team_rpm_limit:bigint";
const SEED_VERIFICATION: &str = r#"INSERT INTO "LiteLLM_VerificationToken" VALUES ('tok-1','alias-1','name-1','team-1','project-1'),('tok-2','alias-2','name-2','missing-team',NULL);
INSERT INTO "LiteLLM_TeamTable" (team_id, spend, max_budget, model_max_budget, tpm_limit, rpm_limit, tpd_limit) VALUES ('team-1',3.5,10.0,'{"gpt-4": 2.0}',100,50,25);
INSERT INTO "LiteLLM_ProjectTable" VALUES ('project-1','proj-alias');"#;
const SEED_LEFT_JOINS: &str = r#"INSERT INTO "LiteLLM_VerificationToken" VALUES ('tok-ok','alias-ok','name-ok','team-1','project-1'),('tok-ghost','alias-ghost','name-ghost','no-team','no-project'),('tok-null',NULL,NULL,NULL,NULL);
INSERT INTO "LiteLLM_TeamTable" (team_id, spend) VALUES ('team-1',1.5);
INSERT INTO "LiteLLM_ProjectTable" VALUES ('project-1','alias-1');
INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, "startTime", model) VALUES ('r1','tok-ok',1.0,CURRENT_DATE,'m'),('r2','tok-ghost',2.0,CURRENT_DATE,'m'),('r3','orphan-key',4.0,CURRENT_DATE,'m');"#;
fn estimate_row(value: Option<i64>) -> RawRow {
    RawRow {
        columns: BTreeMap::from([("reltuples".to_owned(), value)]),
    }
}
fn exists_logs() -> Vec<String> {
    RUNTIME_VIEWS
        .iter()
        .map(|view| format!("{} Exists!", view.name))
        .collect()
}
fn probe_sql(index: usize) -> String {
    format!("SELECT 1 FROM \"{}\" LIMIT 1", RUNTIME_VIEWS[index].name)
}
fn key_row(api_key: &str, alias: Option<&str>, name: Option<&str>, spend: f64) -> KeyRow {
    (
        api_key.to_owned(),
        alias.map(str::to_owned),
        name.map(str::to_owned),
        spend,
    )
}
async fn current_date(client: &Client) -> NaiveDate {
    client
        .query_one("SELECT CURRENT_DATE AS today", &[])
        .await
        .unwrap()
        .get("today")
}
const GLOBAL_SPEND_SQL: &str = r#"SELECT date, spend FROM "MonthlyGlobalSpend" ORDER BY date"#;
const PER_KEY_SQL: &str =
    r#"SELECT date, api_key, spend FROM "MonthlyGlobalSpendPerKey" ORDER BY date, api_key"#;
const PER_USER_KEY_SQL: &str = r#"SELECT date, api_key, "user", spend FROM "MonthlyGlobalSpendPerUserPerKey" ORDER BY date, api_key, "user""#;
const MODELS_SQL: &str = r#"SELECT model, total_spend FROM "Last30dModelsBySpend""#;
const KEYS_SQL: &str =
    r#"SELECT api_key, key_alias, key_name, total_spend FROM "Last30dKeysBySpend""#;
const TAGS_SQL: &str =
    r#"SELECT individual_request_tag, spend_date, log_count, total_spend FROM "DailyTagSpend""#;
const END_USERS_SQL: &str =
    r#"SELECT end_user, total_events, total_spend FROM "Last30dTopEndUsersSpend""#;
const VERIFICATION_SQL: &str = r#"SELECT token, team_spend, team_max_budget, team_tpm_limit, team_rpm_limit FROM "LiteLLM_VerificationTokenView" ORDER BY token"#;
fn spend_of(row: &tokio_postgres::Row) -> (NaiveDate, f64) {
    (row.get(0_usize), row.get(1_usize))
}
fn key_spend_of(row: &tokio_postgres::Row) -> (NaiveDate, String, f64) {
    (row.get(0_usize), row.get(1_usize), row.get(2_usize))
}
fn user_key_spend_of(row: &tokio_postgres::Row) -> (NaiveDate, String, String, f64) {
    (
        row.get(0_usize),
        row.get(1_usize),
        row.get(2_usize),
        row.get(3_usize),
    )
}
fn model_of(row: &tokio_postgres::Row) -> (String, f64) {
    (row.get(0_usize), row.get(1_usize))
}
fn key_of(row: &tokio_postgres::Row) -> KeyRow {
    let api_key = row.get(0_usize);
    let alias = row.get(1_usize);
    let name = row.get(2_usize);
    let spend = row.get(3_usize);
    (api_key, alias, name, spend)
}
fn tag_of(row: &tokio_postgres::Row) -> TagRow {
    let tag = row.get(0_usize);
    let day = row.get(1_usize);
    let count = row.get(2_usize);
    let spend = row.get(3_usize);
    (tag, day, count, spend)
}
fn end_user_of(row: &tokio_postgres::Row) -> EndUserRow {
    (row.get(0_usize), row.get(1_usize), row.get(2_usize))
}
fn verification_head(row: &tokio_postgres::Row) -> (String, Option<f64>, Option<f64>) {
    (row.get(0_usize), row.get(1_usize), row.get(2_usize))
}
fn verification_limits(row: &tokio_postgres::Row) -> (Option<i64>, Option<i64>) {
    (row.get(3_usize), row.get(4_usize))
}
fn verification_of(row: &tokio_postgres::Row) -> VerificationRow {
    let (token, spend, budget) = verification_head(row);
    let (tpm, rpm) = verification_limits(row);
    (token, spend, budget, tpm, rpm)
}
async fn global_spend(client: &Client) -> Vec<(NaiveDate, f64)> {
    let rows = client.query(GLOBAL_SPEND_SQL, &[]).await.unwrap();
    rows.iter().map(spend_of).collect()
}
async fn per_key(client: &Client) -> Vec<(NaiveDate, String, f64)> {
    let rows = client.query(PER_KEY_SQL, &[]).await.unwrap();
    rows.iter().map(key_spend_of).collect()
}
async fn per_user_key(client: &Client) -> Vec<(NaiveDate, String, String, f64)> {
    let rows = client.query(PER_USER_KEY_SQL, &[]).await.unwrap();
    rows.iter().map(user_key_spend_of).collect()
}
async fn models(client: &Client) -> Vec<(String, f64)> {
    let rows = client.query(MODELS_SQL, &[]).await.unwrap();
    rows.iter().map(model_of).collect()
}
async fn keys(client: &Client) -> Vec<KeyRow> {
    let rows = client.query(KEYS_SQL, &[]).await.unwrap();
    rows.iter().map(key_of).collect()
}
async fn tags(client: &Client) -> Vec<TagRow> {
    let rows = client.query(TAGS_SQL, &[]).await.unwrap();
    rows.iter().map(tag_of).collect()
}
async fn end_users(client: &Client) -> Vec<EndUserRow> {
    let rows = client.query(END_USERS_SQL, &[]).await.unwrap();
    rows.iter().map(end_user_of).collect()
}
async fn verification_rows(client: &Client) -> Vec<VerificationRow> {
    let rows = client.query(VERIFICATION_SQL, &[]).await.unwrap();
    rows.iter().map(verification_of).collect()
}
async fn assert_probe_error_propagates(message: &str) {
    let database = ScriptedDatabase::default();
    database.script_probe(Err(FakeFailure::new(message)));
    let result = create_missing_views(&database).await;
    assert_eq!(result.unwrap_err().message, message);
    assert_eq!(database.calls().len(), 1);
    assert!(database.created_ddl().is_empty());
}
async fn assert_marker_creates_views(message: &str) {
    let database = ScriptedDatabase::default();
    database.script_probes_failing(RUNTIME_VIEWS.len(), message);
    database.script_creates_succeed(RUNTIME_VIEWS.len());
    create_missing_views(&database).await.unwrap();
    assert_eq!(database.created_ddl().len(), RUNTIME_VIEWS.len());
}
async fn assert_duplicate_marker_tolerated(message: &str) {
    let database = ScriptedDatabase::default();
    database.script_probes_missing(1);
    database.script_probes_exist(RUNTIME_VIEWS.len().saturating_sub(1));
    database.script_create(Err(FakeFailure::new(message)));
    let (result, logs) = capture_logs(create_missing_views(&database)).await;
    result.unwrap();
    assert_eq!(database.created_ddl().len(), 1);
    let view_name = VERIFICATION_TOKEN_VIEW;
    assert!(logs.contains(&format!(
        "{view_name} already created by a concurrent replica"
    )));
    assert_eq!(database.calls().get(2), Some(&Call::Probe(probe_sql(1))));
}
async fn assert_bootstrap_enabled(estimate: i64) {
    let database = ScriptedDatabase::default();
    database.script_probe(Ok(vec![estimate_row(Some(estimate))]));
    let (result, logs) = capture_logs(should_create_missing_views(&database)).await;
    assert!(result.unwrap());
    assert!(logs.iter().any(|line| line == "Should create views"));
    assert!(
        logs.iter()
            .any(|line| line.starts_with("Estimated Row count of LiteLLM_SpendLogs = "))
    );
}
async fn assert_bootstrap_disabled(rows: Vec<RawRow>) {
    let database = ScriptedDatabase::default();
    database.script_probe(Ok(rows));
    let (result, logs) = capture_logs(should_create_missing_views(&database)).await;
    assert!(!result.unwrap());
    assert!(!logs.iter().any(|line| line == "Should create views"));
}
#[tokio::test]
async fn view_probe_order() {
    let names = [
        "LiteLLM_VerificationTokenView",
        "MonthlyGlobalSpend",
        "Last30dKeysBySpend",
        "Last30dModelsBySpend",
        "MonthlyGlobalSpendPerKey",
        "MonthlyGlobalSpendPerUserPerKey",
        "DailyTagSpend",
        "Last30dTopEndUsersSpend",
    ];
    assert_eq!(RUNTIME_VIEWS.map(|view| view.name), names);
    let database = ScriptedDatabase::default();
    database.script_probes_missing(names.len());
    database.script_creates_succeed(names.len());
    create_missing_views(&database).await.unwrap();
    let expected: Vec<Call> = RUNTIME_VIEWS
        .iter()
        .flat_map(|view| {
            [
                Call::Probe(format!("SELECT 1 FROM \"{}\" LIMIT 1", view.name)),
                Call::Create(view.ddl.to_owned()),
            ]
        })
        .collect();
    assert_eq!(database.calls(), expected);
}
#[tokio::test]
async fn empty_existing_relation_is_kept() {
    let database = ScriptedDatabase::default();
    database.script_probes_exist(RUNTIME_VIEWS.len());
    let (result, logs) = capture_logs(create_missing_views(&database)).await;
    result.unwrap();
    assert_eq!(logs, exists_logs());
    assert!(database.created_ddl().is_empty());
}
#[tokio::test]
async fn missing_message_markers_case_insensitive() {
    assert_marker_creates_views("RELATION DOES NOT EXIST").await;
    assert_marker_creates_views("ERROR: NO SUCH TABLE IN DATABASE").await;
    assert_marker_creates_views("ERROR: UNDEFINED TABLE").await;
}
#[tokio::test]
async fn undefined_function_and_column_not_missing() {
    assert_probe_error_propagates("ERROR: undefined function uuid_generate_v4()").await;
    assert_probe_error_propagates("ERROR: column undefined_col referenced in query").await;
}
#[tokio::test]
async fn privilege_error_propagates() {
    assert_probe_error_propagates("ERROR: permission denied for relation \"LiteLLM_SpendLogs\"")
        .await;
}
#[tokio::test]
async fn duplicate_message_markers_tolerated_without_reprobe() {
    assert_duplicate_marker_tolerated("ERROR: relation already exists").await;
    assert_duplicate_marker_tolerated("ERROR: DUPLICATE OBJECT").await;
    assert_duplicate_marker_tolerated("ERROR: duplicate table").await;
}
#[tokio::test]
async fn other_create_error_stops_sequence() {
    let database = ScriptedDatabase::default();
    database.script_probes_missing(2);
    database.script_create(Ok(()));
    database.script_create(Err(FakeFailure::new(
        "ERROR: syntax error at or near \"FROM\"",
    )));
    let (result, logs) = capture_logs(create_missing_views(&database)).await;
    assert_eq!(
        result.unwrap_err().message,
        "ERROR: syntax error at or near \"FROM\""
    );
    assert_eq!(
        database.calls(),
        vec![
            Call::Probe(probe_sql(0)),
            Call::Create(RUNTIME_VIEWS[0].ddl.to_owned()),
            Call::Probe(probe_sql(1)),
            Call::Create(RUNTIME_VIEWS[1].ddl.to_owned()),
        ]
    );
    let created: Vec<&String> = logs
        .iter()
        .filter(|line| line.contains("Created!"))
        .collect();
    assert_eq!(
        created,
        vec![&format!("{} Created!", RUNTIME_VIEWS[0].name)]
    );
}
#[tokio::test]
async fn cancelled_create_is_not_success() {
    let database = ScriptedDatabase::default();
    database.script_probes_missing(1);
    database.script_create(Err(FakeFailure::new(
        "ERROR: canceling statement due to user request",
    )));
    let (result, logs) = capture_logs(create_missing_views(&database)).await;
    assert_eq!(
        result.unwrap_err().message,
        "ERROR: canceling statement due to user request"
    );
    assert_eq!(database.calls().len(), 2);
    assert!(!logs.iter().any(|line| line.contains("Created!")));
}
#[tokio::test]
async fn zero_and_minus_one_enable_bootstrap() {
    assert_bootstrap_enabled(0).await;
    assert_bootstrap_enabled(-1).await;
}
#[tokio::test]
async fn positive_estimate_disables() {
    assert_bootstrap_disabled(vec![estimate_row(Some(1))]).await;
    assert_bootstrap_disabled(vec![estimate_row(Some(1000))]).await;
}
#[tokio::test]
async fn missing_null_and_malformed_estimate_disable() {
    assert_bootstrap_disabled(Vec::new()).await;
    assert_bootstrap_disabled(vec![RawRow {
        columns: BTreeMap::from([("other".to_owned(), Some(0))]),
    }])
    .await;
    assert_bootstrap_disabled(vec![estimate_row(None)]).await;
    assert_bootstrap_disabled(vec![
        RawRow {
            columns: BTreeMap::from([("other".to_owned(), Some(0))]),
        },
        estimate_row(Some(0)),
    ])
    .await;
}
#[tokio::test]
async fn estimate_error_propagates() {
    let database = ScriptedDatabase::default();
    database.script_probe(Err(FakeFailure::new(
        "ERROR: relation \"LiteLLM_SpendLogs\" does not exist",
    )));
    let (result, logs) = capture_logs(should_create_missing_views(&database)).await;
    assert_eq!(
        result.unwrap_err().message,
        "ERROR: relation \"LiteLLM_SpendLogs\" does not exist"
    );
    assert!(!logs.iter().any(|line| line == "Should create views"));
}
#[tokio::test]
async fn unanalysed_nonempty_table_can_enable() {
    let database = Database::new().await;
    execute(
        &database.client,
        r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, "startTime") VALUES ('a',1.0,CURRENT_DATE),('b',2.0,CURRENT_DATE);"#,
    )
    .await;
    let count: i64 = database
        .client
        .query_one(r#"SELECT COUNT(*) AS rows FROM "LiteLLM_SpendLogs""#, &[])
        .await
        .unwrap()
        .get("rows");
    assert_eq!(count, 2_i64);
    let (result, logs) = capture_logs(should_create_missing_views(&database.client)).await;
    assert!(result.unwrap());
    assert!(logs.iter().any(|line| line == "Should create views"));
    database.close().await;
}
#[tokio::test]
async fn same_named_table_is_not_replaced() {
    let database = Database::new().await;
    execute(
        &database.client,
        r#"CREATE TABLE "MonthlyGlobalSpend" (marker TEXT);
           INSERT INTO "MonthlyGlobalSpend" VALUES ('kept');"#,
    )
    .await;
    create_missing_views(&database.client).await.unwrap();
    let marker: String = database
        .client
        .query_one(r#"SELECT marker FROM "MonthlyGlobalSpend""#, &[])
        .await
        .unwrap()
        .get("marker");
    assert_eq!(marker, "kept");
    let kind: String = database
        .client
        .query_one(
            "SELECT relkind::text AS kind FROM pg_class \
             WHERE relnamespace = current_schema()::regnamespace AND relname = 'MonthlyGlobalSpend'",
            &[],
        )
        .await
        .unwrap()
        .get("kind");
    assert_eq!(kind, "r");
    database.close().await;
}
async fn assert_verification_rows(runtime: &Client, offline: &Client) {
    assert_eq!(
        verification_rows(runtime).await,
        verification_rows(offline).await
    );
    assert_eq!(
        verification_rows(runtime).await,
        vec![
            (
                "tok-1".to_owned(),
                Some(3.5_f64),
                Some(10.0_f64),
                Some(100_i64),
                Some(50_i64)
            ),
            ("tok-2".to_owned(), None, None, None, None),
        ]
    );
    let extra = runtime
        .query_one(
            r#"SELECT team_tpd_limit, project_alias FROM "LiteLLM_VerificationTokenView" WHERE token = 'tok-1'"#,
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        (
            extra.get::<_, Option<i64>>("team_tpd_limit"),
            extra.get::<_, Option<String>>("project_alias")
        ),
        (Some(25_i64), Some("proj-alias".to_owned()))
    );
}
#[tokio::test]
async fn postgres_runtime_and_offline_verification_columns() {
    let runtime = Database::new().await;
    let offline = Database::new().await;
    execute(&runtime.client, SEED_VERIFICATION).await;
    execute(&offline.client, SEED_VERIFICATION).await;
    create_missing_views(&runtime.client).await.unwrap();
    create_offline_views(&offline).await;
    assert_view_columns(&runtime.client, VERIFICATION_TOKEN_VIEW, RUNTIME_COLUMNS).await;
    assert_view_columns(&offline.client, VERIFICATION_TOKEN_VIEW, OFFLINE_COLUMNS).await;
    assert_verification_rows(&runtime.client, &offline.client).await;
    runtime.close().await;
    offline.close().await;
}
#[tokio::test]
async fn runtime_keeps_existing_offline_verification_view() {
    let database = Database::new().await;
    execute(&database.client, SEED_VERIFICATION).await;
    create_offline_views(&database).await;
    create_missing_views(&database.client).await.unwrap();
    assert_view_columns(&database.client, VERIFICATION_TOKEN_VIEW, OFFLINE_COLUMNS).await;
    database.close().await;
}
#[tokio::test]
async fn left_joins_keep_missing_team_project_and_key() {
    let database = Database::new().await;
    execute(&database.client, SEED_LEFT_JOINS).await;
    create_missing_views(&database.client).await.unwrap();
    assert_eq!(
        verification_rows(&database.client).await,
        vec![
            ("tok-ghost".to_owned(), None, None, None, None),
            ("tok-null".to_owned(), None, None, None, None),
            ("tok-ok".to_owned(), Some(1.5_f64), None, None, None),
        ]
    );
    let extra = database
        .client
        .query_one(
            r#"SELECT project_alias FROM "LiteLLM_VerificationTokenView" WHERE token = 'tok-ok'"#,
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        extra.get::<_, Option<String>>("project_alias"),
        Some("alias-1".to_owned())
    );
    assert_eq!(
        keys(&database.client).await,
        vec![
            key_row("orphan-key", None, None, 4.0),
            key_row("tok-ghost", Some("alias-ghost"), Some("name-ghost"), 2.0),
            key_row("tok-ok", Some("alias-ok"), Some("name-ok"), 1.0),
        ]
    );
    database.close().await;
}
#[tokio::test]
async fn inclusive_thirty_day_midnight() {
    let database = Database::new().await;
    execute(
        &database.client,
        r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, "startTime", model) VALUES ('boundary','k',1.0,CURRENT_DATE - INTERVAL '30 days','m'),('before','k',2.0,CURRENT_DATE - INTERVAL '30 days' - INTERVAL '1 second','m'),('after','k',4.0,CURRENT_DATE - INTERVAL '30 days' + INTERVAL '1 second','m'),('today','k',8.0,CURRENT_DATE + INTERVAL '1 hour','m'),('old','k',16.0,CURRENT_DATE - INTERVAL '31 days','m');"#,
    )
    .await;
    create_missing_views(&database.client).await.unwrap();
    let today = current_date(&database.client).await;
    let boundary = today.checked_sub_days(Days::new(30)).unwrap();
    assert_eq!(
        global_spend(&database.client).await,
        vec![(boundary, 5.0_f64), (today, 8.0_f64)]
    );
    database.close().await;
}
#[tokio::test]
async fn future_and_negative_spend_included() {
    let database = Database::new().await;
    execute(
        &database.client,
        r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, "startTime", model) VALUES ('today','k',2.5,CURRENT_DATE,'m'),('future','k',-3.5,CURRENT_DATE + INTERVAL '5 days','m'),('old','k',9.0,CURRENT_DATE - INTERVAL '31 days','m');"#,
    )
    .await;
    create_missing_views(&database.client).await.unwrap();
    let today = current_date(&database.client).await;
    let future = today.checked_add_days(Days::new(5)).unwrap();
    assert_eq!(
        global_spend(&database.client).await,
        vec![(today, 2.5_f64), (future, -3.5_f64)]
    );
    assert_eq!(
        keys(&database.client).await,
        vec![key_row("k", None, None, -1.0)]
    );
    database.close().await;
}
async fn assert_monthly_rows(client: &Client) {
    let today = current_date(client).await;
    let yesterday = today.checked_sub_days(Days::new(1)).unwrap();
    assert_eq!(
        global_spend(client).await,
        vec![(yesterday, 8.0_f64), (today, 7.0_f64)]
    );
    assert_eq!(
        per_key(client).await,
        vec![
            (yesterday, "key-A".to_owned(), 8.0_f64),
            (today, "key-A".to_owned(), 3.0_f64),
            (today, "key-B".to_owned(), 4.0_f64),
        ]
    );
    assert_eq!(
        per_user_key(client).await,
        vec![
            (yesterday, "key-A".to_owned(), "u1".to_owned(), 8.0_f64),
            (today, "key-A".to_owned(), "u1".to_owned(), 1.0_f64),
            (today, "key-A".to_owned(), "u2".to_owned(), 2.0_f64),
            (today, "key-B".to_owned(), "u1".to_owned(), 4.0_f64),
        ]
    );
}
#[tokio::test]
async fn monthly_views_group_by_exact_dimensions() {
    let database = Database::new().await;
    execute(
        &database.client,
        r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, "startTime", model, "user") VALUES ('r1','key-A',1.0,CURRENT_DATE,'m','u1'),('r2','key-A',2.0,CURRENT_DATE,'m','u2'),('r3','key-B',4.0,CURRENT_DATE,'m','u1'),('r4','key-A',8.0,CURRENT_DATE - INTERVAL '1 day','m','u1');"#,
    )
    .await;
    create_missing_views(&database.client).await.unwrap();
    assert_view_columns(
        &database.client,
        MONTHLY_GLOBAL_SPEND,
        "date:date,spend:double precision",
    )
    .await;
    assert_view_columns(
        &database.client,
        MONTHLY_GLOBAL_SPEND_PER_KEY,
        "date:date,spend:double precision,api_key:text",
    )
    .await;
    assert_view_columns(
        &database.client,
        MONTHLY_USER_KEY_SPEND,
        "date:date,spend:double precision,api_key:text,user:text",
    )
    .await;
    assert_monthly_rows(&database.client).await;
    database.close().await;
}
#[tokio::test]
async fn models_exclude_empty_and_null() {
    let database = Database::new().await;
    execute(
        &database.client,
        r#"ALTER TABLE "LiteLLM_SpendLogs" ALTER COLUMN model DROP NOT NULL;
INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, "startTime", model) VALUES ('r1',1.0,CURRENT_DATE,''),('r2',2.0,CURRENT_DATE,''),('r3',4.0,CURRENT_DATE,NULL),('r4',8.0,CURRENT_DATE,'gpt-4'),('r5',5.0,CURRENT_DATE - INTERVAL '2 days','claude-3');"#,
    )
    .await;
    create_missing_views(&database.client).await.unwrap();
    assert_view_columns(
        &database.client,
        LAST_30D_MODELS_BY_SPEND,
        "model:text,total_spend:double precision",
    )
    .await;
    assert_eq!(
        models(&database.client).await,
        vec![
            ("gpt-4".to_owned(), 8.0_f64),
            ("claude-3".to_owned(), 5.0_f64)
        ]
    );
    database.close().await;
}
#[tokio::test]
async fn keys_group_alias_and_name() {
    let database = Database::new().await;
    execute(
        &database.client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token, key_alias, key_name) VALUES ('key-1','alias-1','name-1'),('key-2','alias-2','name-2');
INSERT INTO "LiteLLM_SpendLogs" (request_id, api_key, spend, "startTime", model) VALUES ('r1','key-1',1.0,CURRENT_DATE,'m'),('r2','key-1',2.0,CURRENT_DATE,'m'),('r3','key-2',5.0,CURRENT_DATE,'m'),('r4','deleted-key',9.0,CURRENT_DATE - INTERVAL '3 days','m');"#,
    )
    .await;
    create_missing_views(&database.client).await.unwrap();
    assert_view_columns(
        &database.client,
        LAST_30D_KEYS_BY_SPEND,
        "api_key:text,key_alias:text,key_name:text,total_spend:double precision",
    )
    .await;
    assert_eq!(
        keys(&database.client).await,
        vec![
            key_row("deleted-key", None, None, 9.0),
            key_row("key-2", Some("alias-2"), Some("name-2"), 5.0),
            key_row("key-1", Some("alias-1"), Some("name-1"), 3.0),
        ]
    );
    database.close().await;
}
#[tokio::test]
async fn tags_all_dates_and_duplicate_occurrences() {
    let database = Database::new().await;
    execute(
        &database.client,
        r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, "startTime", model, request_tags) VALUES ('r1',1.0,CURRENT_DATE,'m','["a", "b"]'),('r2',2.0,CURRENT_DATE - INTERVAL '1 day','m','["a"]'),('r3',4.0,CURRENT_DATE - INTERVAL '100 days','m','["a"]'),('r4',0.5,CURRENT_DATE,'m','["a", "a"]');"#,
    )
    .await;
    create_missing_views(&database.client).await.unwrap();
    assert_view_columns(
        &database.client,
        DAILY_TAG_SPEND,
        "individual_request_tag:text,spend_date:date,log_count:bigint,total_spend:double precision",
    )
    .await;
    let today = current_date(&database.client).await;
    let yesterday = today.checked_sub_days(Days::new(1)).unwrap();
    let old = today.checked_sub_days(Days::new(100)).unwrap();
    let mut rows = tags(&database.client).await;
    rows.sort_by_key(|row| (row.0.clone(), row.1, row.2));
    assert_eq!(
        rows,
        vec![
            ("a".to_owned(), old, 1_i64, 4.0_f64),
            ("a".to_owned(), yesterday, 1_i64, 2.0_f64),
            ("a".to_owned(), today, 3_i64, 2.0_f64),
            ("b".to_owned(), today, 1_i64, 1.0_f64),
        ]
    );
    database.close().await;
}
async fn assert_daily_tags_reject(client: &Client, request_tags: &str, message: &str) {
    execute(
        client,
        &format!(
            r#"DELETE FROM "LiteLLM_SpendLogs";
               INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, "startTime", model, request_tags) VALUES ('tagged',4.0,CURRENT_DATE,'m','{request_tags}');"#
        ),
    )
    .await;
    let error = client
        .query(r#"SELECT individual_request_tag FROM "DailyTagSpend""#, &[])
        .await
        .unwrap_err();
    assert!(error.as_db_error().unwrap().message().contains(message));
}
#[tokio::test]
async fn tags_empty_null_and_scalar() {
    let database = Database::new().await;
    execute(
        &database.client,
        r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, "startTime", model, request_tags) VALUES ('empty',1.0,CURRENT_DATE,'m','[]'),('null-tags',2.0,CURRENT_DATE,'m',NULL);"#,
    )
    .await;
    create_missing_views(&database.client).await.unwrap();
    assert!(tags(&database.client).await.is_empty());
    assert_daily_tags_reject(
        &database.client,
        r#""just-a-string""#,
        "cannot extract elements from a scalar",
    )
    .await;
    assert_daily_tags_reject(
        &database.client,
        r#"{"a": 1}"#,
        "cannot extract elements from an object",
    )
    .await;
    database.close().await;
}
async fn seed_end_user_rows(client: &Client, database_user: &str, end_user: &str) {
    client
        .execute(
            r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, "startTime", model, "user", request_tags, end_user) VALUES ('r1',1.0,CURRENT_DATE,'m',$1,'[]',$1)"#,
            &[&end_user],
        )
        .await
        .unwrap();
    client
        .execute(
            r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, "startTime", model, "user", request_tags, end_user) VALUES ('r2',2.0,CURRENT_DATE,'m',$1,'[]',$2)"#,
            &[&database_user, &end_user],
        )
        .await
        .unwrap();
    client
        .execute(
            r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, "startTime", model, "user", request_tags, end_user) VALUES ('r3',4.0,CURRENT_DATE,'m','mallory','[]',$1)"#,
            &[&database_user],
        )
        .await
        .unwrap();
    execute(
        client,
        r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, "startTime", model, "user", request_tags, end_user) VALUES ('r4',8.0,CURRENT_DATE,'m','mallory','[]',NULL),('r5',16.0,CURRENT_DATE,'m','mallory','[]','');"#,
    )
    .await;
}
#[tokio::test]
async fn end_users_compare_database_user_not_row_user() {
    let database = Database::new().await;
    let database_user: String = database
        .client
        .query_one("SELECT current_user::text AS database_user", &[])
        .await
        .unwrap()
        .get("database_user");
    let end_user = format!("{database_user}-end-user");
    seed_end_user_rows(&database.client, &database_user, &end_user).await;
    create_missing_views(&database.client).await.unwrap();
    assert_view_columns(
        &database.client,
        LAST_30D_TOP_END_USERS_SPEND,
        "end_user:text,total_events:bigint,total_spend:double precision",
    )
    .await;
    assert_eq!(
        end_users(&database.client).await,
        vec![(end_user.clone(), 2_i64, 3.0_f64)]
    );
    database.close().await;
}
#[tokio::test]
async fn end_users_descending_top_hundred() {
    let database = Database::new().await;
    let database_user: String = database
        .client
        .query_one("SELECT current_user::text AS database_user", &[])
        .await
        .unwrap()
        .get("database_user");
    assert!(!database_user.starts_with("end-user-"));
    let values: Vec<String> = (1..=101_usize)
        .map(|rank| format!("('r{rank}',{rank}.0,CURRENT_DATE,'m','','[]','end-user-{rank}')"))
        .collect();
    execute(
        &database.client,
        &format!(
            r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, spend, "startTime", model, "user", request_tags, end_user) VALUES {}"#,
            values.join(",")
        ),
    )
    .await;
    create_missing_views(&database.client).await.unwrap();
    let rows = end_users(&database.client).await;
    assert_eq!(rows.len(), 100_usize);
    assert_eq!(
        rows.first(),
        Some(&("end-user-101".to_owned(), 1_i64, 101.0_f64))
    );
    assert_eq!(
        rows.last(),
        Some(&("end-user-2".to_owned(), 1_i64, 2.0_f64))
    );
    let spends: Vec<f64> = rows.iter().map(|row| row.2).collect();
    assert!(
        spends
            .windows(2)
            .all(|pair| matches!(pair, [left, right] if left > right))
    );
    database.close().await;
}
#[tokio::test]
async fn postgres_concurrent_create_race() {
    let database = Database::new().await;
    let (second, second_tasks) = database.another_client().await;
    let probe = format!("SELECT 1 FROM \"{VERIFICATION_TOKEN_VIEW}\" LIMIT 1");
    database.client.query(&probe, &[]).await.unwrap_err();
    second.query(&probe, &[]).await.unwrap_err();
    create_view_tolerating_race(
        &database.client,
        VERIFICATION_TOKEN_VIEW,
        RUNTIME_VERIFICATION_VIEW_SQL,
    )
    .await
    .unwrap();
    let (loser, logs) = capture_logs(create_view_tolerating_race(
        &second,
        VERIFICATION_TOKEN_VIEW,
        RUNTIME_VERIFICATION_VIEW_SQL,
    ))
    .await;
    loser.unwrap();
    let view_name = VERIFICATION_TOKEN_VIEW;
    assert!(logs.contains(&format!(
        "{view_name} already created by a concurrent replica"
    )));
    drop(second);
    drop(second_tasks);
    assert_eq!(
        created_view_names(&database.client).await,
        vec![VERIFICATION_TOKEN_VIEW.to_owned()]
    );
    let definition: String = database
        .client
        .query_one(
            "SELECT pg_get_viewdef('\"LiteLLM_VerificationTokenView\"'::regclass) AS definition",
            &[],
        )
        .await
        .unwrap()
        .get("definition");
    assert!(definition.contains("project_alias"));
    database.close().await;
}
#[tokio::test]
async fn postgres_failed_create_preserves_previous_views() {
    let database = Database::new().await;
    execute(
        &database.client,
        r#"ALTER TABLE "LiteLLM_SpendLogs" DROP COLUMN spend"#,
    )
    .await;
    let error = create_missing_views(&database.client).await.unwrap_err();
    assert!(error.as_db_error().unwrap().message().contains("spend"));
    assert_eq!(
        created_view_names(&database.client).await,
        vec![VERIFICATION_TOKEN_VIEW.to_owned()]
    );
    database.close().await;
}

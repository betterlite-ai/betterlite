#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str::FromStr as _;

use chrono::{NaiveDate, NaiveDateTime};
use rstest::rstest;
use rust_decimal::Decimal;
use tokio_postgres::Row;

use crate::budget_permission_rows::JsonData;

use super::cells::{RowError, VerificationTokenRow};
use super::fixtures::RepairCase;
use super::migration::{ImportEvent, ImportFailure, ImportReport, import_verification_tokens};
use super::unassigned::{UnassignedTeamError, apply_db_fixes};
use super::{DecodeError, MAX_CSV_BYTES, MAX_JSON_DEPTH, decode_verification_csv};

// the CSV columns LiteLLM's migrate_keys.py requires, in the order it reads them
const COLUMNS: [&str; 31] = [
    "token",
    "key_name",
    "key_alias",
    "soft_budget_cooldown",
    "spend",
    "expires",
    "models",
    "aliases",
    "config",
    "user_id",
    "team_id",
    "permissions",
    "max_parallel_requests",
    "metadata",
    "tpm_limit",
    "rpm_limit",
    "max_budget",
    "budget_duration",
    "budget_reset_at",
    "allowed_cache_controls",
    "model_spend",
    "model_max_budget",
    "budget_id",
    "blocked",
    "created_at",
    "updated_at",
    "allowed_routes",
    "object_permission_id",
    "created_by",
    "updated_by",
    "organization_id",
];

const FULL_CSV: &str = concat!(
    "token,key_name,key_alias,soft_budget_cooldown,spend,expires,models,aliases,config,user_id,team_id,permissions,max_parallel_requests,metadata,tpm_limit,rpm_limit,max_budget,budget_duration,budget_reset_at,allowed_cache_controls,model_spend,model_max_budget,budget_id,blocked,created_at,updated_at,allowed_routes,object_permission_id,created_by,updated_by,organization_id\n",
    "tok-full,name-one,alias-one,TRUE,1.25,2026-01-02T03:04:05Z,\"{gpt-4,claude}\",\"{\"\"tier\"\":\"\"pro\"\"}\",\"{\"\"a\"\":1,\"\"nested\"\":{\"\"deep\"\":true}}\",user-1,team-a,\"{\"\"perm\"\":\"\"read\"\"}\",7,\"{\"\"meta\"\":1}\",100,200,5.5,1d,2026-02-03T04:05:06Z,\"{default-models,admin}\",\"{\"\"m\"\":2}\",\"{\"\"limit\"\":3}\",budget-1,false,2026-03-04T05:06:07Z,2026-03-05T06:07:08Z,\"{route-a}\",perm-obj-1,creator,updater,org-1\n",
    "tok-minimal\n",
    "tok-existing,renamed\n",
);

fn quote_cell(cell: &str) -> String {
    format!("\"{}\"", cell.replace('"', "\"\""))
}

fn cell_for<'a>(cells: &[(&str, &'a str)], name: &str) -> Option<&'a str> {
    cells
        .iter()
        .find(|entry| entry.0 == name)
        .map(|entry| entry.1)
}

fn csv_data(rows: &[&[(&str, &str)]]) -> String {
    let records = rows
        .iter()
        .map(|row| {
            COLUMNS
                .iter()
                .enumerate()
                .map(|(position, name)| {
                    let fallback = if position == 0 { "tok-1" } else { "" };
                    quote_cell(cell_for(row, name).unwrap_or(fallback))
                })
                .collect::<Vec<String>>()
                .join(",")
        })
        .collect::<Vec<String>>()
        .join("\n");
    format!("{}\n{records}\n", COLUMNS.join(","))
}

fn decode_cell(column: &str, cell: &str) -> Result<VerificationTokenRow, RowError> {
    let data = csv_data(&[&[(column, cell)]]);
    let mut decoded = decode_verification_csv(data.as_bytes()).unwrap();
    decoded.rows.remove(0).outcome
}

fn parsed_json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn moment(day: (i32, u32, u32), time: (u32, u32, u32)) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(day.0, day.1, day.2)
        .unwrap()
        .and_hms_opt(time.0, time.1, time.2)
        .unwrap()
}

fn json_depth_csv(depth: usize) -> String {
    let document = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    csv_data(&[&[("config", &document)]])
}

fn optional_decimal(row: &Row, column: &str) -> Option<Decimal> {
    let text: Option<String> = row.get(column);
    text.map(|value| Decimal::from_str(&value).unwrap())
}

fn json_column(row: &Row, column: &str) -> JsonData {
    let text: String = row.get(column);
    parsed_json(&text)
}

#[test]
fn csv_json_null_coercion() {
    let data = csv_data(&[
        &[
            ("token", "tok-1"),
            ("config", r#"{"a":1,"nested":[true,null]}"#),
            ("metadata", ""),
            ("aliases", "NULL"),
            ("permissions", "{}"),
        ],
        &[
            ("token", "tok-2"),
            ("config", "null"),
            ("metadata", r#"{"b":"text"}"#),
            ("aliases", ""),
            ("permissions", ""),
        ],
    ]);
    let decoded = decode_verification_csv(data.as_bytes()).unwrap();
    let first = decoded.rows[0].outcome.as_ref().unwrap();
    assert_eq!(
        first.config,
        Some(parsed_json(r#"{"a":1,"nested":[true,null]}"#))
    );
    assert_eq!(first.metadata, None);
    assert_eq!(first.aliases, None);
    assert_eq!(first.permissions, Some(parsed_json("{}")));
    let second = decoded.rows[1].outcome.as_ref().unwrap();
    assert_eq!(second.config, Some(JsonData::Null));
    assert_eq!(second.metadata, Some(parsed_json(r#"{"b":"text"}"#)));
    assert_eq!(second.aliases, None);
    assert_eq!(second.permissions, None);
}

#[rstest]
#[case::decimal("spend", "abc", "decimal")]
#[case::non_finite("spend", "inf", "decimal")]
#[case::json("config", "{oops", "JSON")]
#[case::duplicate_json_key("config", r#"{"a":1,"a":2}"#, "JSON")]
fn csv_invalid_value(#[case] column: &str, #[case] cell: &str, #[case] kind: &str) {
    let error = decode_cell(column, cell).unwrap_err();
    assert!(
        matches!(&error, RowError::InvalidValue { name, kind: actual, .. } if *name == column && *actual == kind)
    );
}

#[test]
fn csv_failed_rows_keep_their_error_and_other_rows() {
    let data = csv_data(&[
        &[("token", "tok-1"), ("spend", "1.5")],
        &[("token", "tok-2"), ("spend", "oops")],
        &[("token", "tok-3")],
    ]);
    let decoded = decode_verification_csv(data.as_bytes()).unwrap();
    assert_eq!(decoded.rows.len(), 3);
    assert!(decoded.rows[0].outcome.is_ok());
    assert!(matches!(
        decoded.rows[1].outcome,
        Err(RowError::InvalidValue {
            kind: "decimal",
            ..
        })
    ));
    assert!(decoded.rows[2].outcome.is_ok());
    assert_eq!(decoded.rows[1].token.as_deref(), Some("tok-2"));
}

#[rstest]
#[case::lower("true", Some(true))]
#[case::mixed_case("TRUE", Some(true))]
#[case::other("yes", Some(false))]
#[case::empty("", None)]
#[case::null("NULL", None)]
fn csv_boolean_cells(#[case] cell: &str, #[case] expected: Option<bool>) {
    let row = decode_cell("blocked", cell).unwrap();
    assert_eq!(row.blocked, expected);
}

#[rstest]
#[case::digits("42", Some(42))]
#[case::zero("0", Some(0))]
#[case::large("9007199254740993", Some(9_007_199_254_740_993))]
#[case::negative("-5", None)]
#[case::decimal_point("1.5", None)]
#[case::word("many", None)]
#[case::unicode_digits("١٢", None)]
#[case::null("NULL", None)]
fn csv_integer_cells(#[case] cell: &str, #[case] expected: Option<i64>) {
    let row = decode_cell("tpm_limit", cell).unwrap();
    assert_eq!(row.tpm_limit, expected);
}

#[test]
fn csv_integer_overflow_fails_the_row() {
    let error = decode_cell("tpm_limit", "99999999999999999999").unwrap_err();
    assert!(matches!(
        error,
        RowError::InvalidValue {
            kind: "integer",
            ..
        }
    ));
}

#[rstest]
#[case::zulu("2026-01-02T03:04:05Z", Some("2026-01-02 03:04:05"))]
#[case::offset("2026-01-02T03:04:05+02:00", Some("2026-01-02 01:04:05"))]
#[case::offset_compact("2026-01-02T03:04:05-0530", Some("2026-01-02 08:34:05"))]
#[case::naive("2026-01-02 03:04:05", Some("2026-01-02 03:04:05"))]
#[case::minutes("2026-01-02T03:04", Some("2026-01-02 03:04:00"))]
#[case::date("2026-01-02", Some("2026-01-02 00:00:00"))]
#[case::compact_date("20260102", Some("2026-01-02 00:00:00"))]
#[case::fraction("2026-01-02T03:04:05.123456", Some("2026-01-02 03:04:05.123456"))]
#[case::garbage("not-a-time", None)]
fn csv_timestamp_cells(#[case] cell: &str, #[case] expected: Option<&str>) {
    let row = decode_cell("expires", cell).unwrap();
    let rendered = row.expires.map(|value| value.to_string());
    assert_eq!(rendered.as_deref(), expected);
}

#[rstest]
#[case::single("{only}", Some("only"))]
#[case::spaced("{a, b ,c}", Some("a|b|c"))]
#[case::unbraced("plain", Some(""))]
#[case::empty_braces("{}", Some(""))]
#[case::null("NULL", None)]
fn csv_string_array_cells(#[case] cell: &str, #[case] expected: Option<&str>) {
    let row = decode_cell("models", cell).unwrap();
    let joined = row.models.map(|items| items.join("|"));
    assert_eq!(joined.as_deref(), expected);
}

#[test]
fn csv_short_rows_take_column_defaults() {
    let data = format!("{}\ntok-1\n", COLUMNS.join(","));
    let decoded = decode_verification_csv(data.as_bytes()).unwrap();
    let row = decoded.rows[0].outcome.as_ref().unwrap();
    assert_eq!(row.key_name, None);
    assert_eq!(row.spend, None);
    assert_eq!(row.models, None);
    assert_eq!(decoded.rows[0].token.as_deref(), Some("tok-1"));
}

#[test]
fn csv_absent_columns_fail_the_row() {
    let decoded = decode_verification_csv(b"token,spend\ntok-1,1.5\n").unwrap();
    let error = decoded.rows[0].outcome.as_ref().unwrap_err();
    assert!(matches!(
        error,
        RowError::MissingColumn { name: "key_name" }
    ));
    assert_eq!(decoded.rows[0].token.as_deref(), Some("tok-1"));
}

#[test]
fn csv_absent_token_column_fails_each_row() {
    let decoded = decode_verification_csv(b"spend\ntok-1\n").unwrap();
    let error = decoded.rows[0].outcome.as_ref().unwrap_err();
    assert!(matches!(error, RowError::MissingColumn { name: "token" }));
    assert_eq!(decoded.rows[0].token, None);
}

#[test]
fn csv_duplicate_headers_take_the_last_column() {
    let mut header = COLUMNS.join(",");
    header.push_str(",spend");
    let mut cells: Vec<String> = COLUMNS
        .iter()
        .enumerate()
        .map(|(position, name)| {
            if position == 0 {
                quote_cell("tok-1")
            } else if *name == "spend" {
                quote_cell("1.5")
            } else {
                quote_cell("")
            }
        })
        .collect();
    cells.push(quote_cell("2.25"));
    let data = format!("{header}\n{}\n", cells.join(","));
    let decoded = decode_verification_csv(data.as_bytes()).unwrap();
    let row = decoded.rows[0].outcome.as_ref().unwrap();
    assert_eq!(row.spend, Some(Decimal::new(225, 2)));
}

#[test]
fn csv_extra_cells_are_ignored() {
    let mut cells = COLUMNS
        .iter()
        .enumerate()
        .map(|(position, _)| quote_cell(if position == 0 { "tok-1" } else { "" }))
        .collect::<Vec<String>>();
    cells.push(quote_cell("extra-cell"));
    let data = format!("{}\n{}\n", COLUMNS.join(","), cells.join(","));
    let decoded = decode_verification_csv(data.as_bytes()).unwrap();
    let row = decoded.rows[0].outcome.as_ref().unwrap();
    assert_eq!(row.token, "tok-1");
    assert_eq!(row.key_name, None);
}

#[test]
fn native_new_otherwise_unbounded_byte_decoders_reject_over_1048576_bytes_or_jso() {
    let oversized = vec![b'a'; MAX_CSV_BYTES + 1];
    assert!(matches!(
        decode_verification_csv(&oversized),
        Err(DecodeError::TooLarge { len }) if len == MAX_CSV_BYTES + 1
    ));
    let exact = vec![b'a'; MAX_CSV_BYTES];
    assert!(!matches!(
        decode_verification_csv(&exact),
        Err(DecodeError::TooLarge { .. })
    ));
    let at_limit = decode_verification_csv(json_depth_csv(MAX_JSON_DEPTH).as_bytes()).unwrap();
    let over_limit = decode_verification_csv(json_depth_csv(MAX_JSON_DEPTH + 1).as_bytes());
    assert!(at_limit.rows[0].outcome.is_ok());
    assert!(matches!(over_limit, Err(DecodeError::TooDeep)));
    assert!(matches!(
        decode_verification_csv(&[0xff, 0xfe]),
        Err(DecodeError::InvalidUtf8(_))
    ));
}

#[rstest]
fn replays_the_database_repair_corpus(#[files("fuzz/corpus/database_repair/*")] path: PathBuf) {
    drop(decode_verification_csv(&fs::read(path).unwrap()));
}

#[tokio::test]
async fn unassigned_team_repair() {
    let postgres = RepairCase::provision().await;
    postgres.seed_token("tok-a", Some("team-a")).await;
    postgres.seed_token("tok-b", Some("team-b")).await;
    postgres.seed_token("tok-null", None).await;
    postgres.seed_spend_log("req-a", "tok-a", None).await;
    postgres.seed_spend_log("req-b", "tok-b", None).await;
    postgres.seed_spend_log("req-null", "tok-null", None).await;
    postgres
        .seed_spend_log("req-missing", "tok-unknown", None)
        .await;
    postgres
        .seed_spend_log("req-manual", "tok-a", Some("team-manual"))
        .await;
    let updated = apply_db_fixes(&postgres.client).await.unwrap();
    assert_eq!(updated, 3);
    assert_eq!(
        postgres.spend_team("req-a").await.as_deref(),
        Some("team-a")
    );
    assert_eq!(
        postgres.spend_team("req-b").await.as_deref(),
        Some("team-b")
    );
    assert_eq!(postgres.spend_team("req-null").await, None);
    assert_eq!(postgres.spend_team("req-missing").await, None);
    assert_eq!(
        postgres.spend_team("req-manual").await.as_deref(),
        Some("team-manual")
    );
    postgres.close().await;
}

#[tokio::test]
async fn postgres_pinned_rows() {
    let postgres = RepairCase::provision().await;
    postgres.seed_token("tok-existing", Some("team-old")).await;
    let report = import_csv(&postgres, FULL_CSV).await;
    assert_eq!(report.migrated(), 2);
    assert_eq!(report.skipped(), 1);
    assert_eq!(report.failed(), 0);
    assert_full_row(&postgres).await;
    assert_minimal_row(&postgres).await;
    let existing = postgres.token_row("tok-existing").await;
    assert_eq!(
        existing.get::<_, Option<String>>("key_name").as_deref(),
        Some("seed-tok-existing")
    );
    assert_eq!(
        existing.get::<_, Option<String>>("team_id").as_deref(),
        Some("team-old")
    );
    postgres.close().await;
}

async fn import_csv(postgres: &RepairCase, csv: &str) -> ImportReport {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tokens.csv");
    fs::write(&path, csv).unwrap();
    import_verification_tokens(&postgres.client, &path)
        .await
        .unwrap()
}

async fn assert_full_row(postgres: &RepairCase) {
    let row = postgres.token_row("tok-full").await;
    assert_full_identity(&row);
    assert_full_budgets(&row);
    assert_full_objects(&row);
}

fn assert_full_identity(row: &Row) {
    assert_eq!(
        row.get::<_, Option<String>>("key_name").as_deref(),
        Some("name-one")
    );
    assert_eq!(
        row.get::<_, Option<String>>("key_alias").as_deref(),
        Some("alias-one")
    );
    assert_eq!(
        row.get::<_, Option<String>>("user_id").as_deref(),
        Some("user-1")
    );
    assert_eq!(
        row.get::<_, Option<String>>("team_id").as_deref(),
        Some("team-a")
    );
    assert_eq!(
        row.get::<_, Option<String>>("budget_id").as_deref(),
        Some("budget-1")
    );
    assert_eq!(
        row.get::<_, Option<String>>("organization_id").as_deref(),
        Some("org-1")
    );
    assert_eq!(
        row.get::<_, Option<String>>("object_permission_id")
            .as_deref(),
        Some("perm-obj-1")
    );
    assert_eq!(
        row.get::<_, Option<String>>("created_by").as_deref(),
        Some("creator")
    );
    assert_eq!(
        row.get::<_, Option<String>>("updated_by").as_deref(),
        Some("updater")
    );
    assert!(row.get::<_, bool>("soft_budget_cooldown"));
    assert_eq!(row.get::<_, Option<bool>>("blocked"), Some(false));
}

fn assert_full_budgets(row: &Row) {
    assert_eq!(optional_decimal(row, "spend"), Some(Decimal::new(125, 2)));
    assert_eq!(
        optional_decimal(row, "max_budget"),
        Some(Decimal::new(55, 1))
    );
    assert_eq!(row.get::<_, Option<i32>>("max_parallel_requests"), Some(7));
    assert_eq!(row.get::<_, Option<i64>>("tpm_limit"), Some(100));
    assert_eq!(row.get::<_, Option<i64>>("rpm_limit"), Some(200));
    assert_eq!(
        row.get::<_, Option<String>>("budget_duration").as_deref(),
        Some("1d")
    );
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("expires"),
        Some(moment((2026, 1, 2), (3, 4, 5)))
    );
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("budget_reset_at"),
        Some(moment((2026, 2, 3), (4, 5, 6)))
    );
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("created_at"),
        Some(moment((2026, 3, 4), (5, 6, 7)))
    );
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("updated_at"),
        Some(moment((2026, 3, 5), (6, 7, 8)))
    );
}

fn assert_full_objects(row: &Row) {
    assert_eq!(
        json_column(row, "aliases"),
        parsed_json(r#"{"tier":"pro"}"#)
    );
    assert_eq!(
        json_column(row, "config"),
        parsed_json(r#"{"a":1,"nested":{"deep":true}}"#)
    );
    assert_eq!(
        json_column(row, "permissions"),
        parsed_json(r#"{"perm":"read"}"#)
    );
    assert_eq!(json_column(row, "metadata"), parsed_json(r#"{"meta":1}"#));
    assert_eq!(json_column(row, "model_spend"), parsed_json(r#"{"m":2}"#));
    assert_eq!(
        json_column(row, "model_max_budget"),
        parsed_json(r#"{"limit":3}"#)
    );
    assert_eq!(
        row.get::<_, Option<Vec<String>>>("models"),
        Some(vec!["gpt-4".to_owned(), "claude".to_owned()])
    );
    assert_eq!(
        row.get::<_, Option<Vec<String>>>("allowed_cache_controls"),
        Some(vec!["default-models".to_owned(), "admin".to_owned()])
    );
    assert_eq!(
        row.get::<_, Option<Vec<String>>>("allowed_routes"),
        Some(vec!["route-a".to_owned()])
    );
}

async fn assert_minimal_row(postgres: &RepairCase) {
    let row = postgres.token_row("tok-minimal").await;
    assert_eq!(row.get::<_, Option<String>>("key_name"), None);
    assert_eq!(row.get::<_, Option<String>>("team_id"), None);
    assert_eq!(row.get::<_, Option<String>>("budget_duration"), None);
    assert_eq!(row.get::<_, Option<bool>>("blocked"), None);
    assert_eq!(row.get::<_, Option<i64>>("tpm_limit"), None);
    assert_eq!(row.get::<_, Option<NaiveDateTime>>("expires"), None);
    assert_eq!(row.get::<_, Option<Vec<String>>>("models"), None);
    assert!(!row.get::<_, bool>("soft_budget_cooldown"));
    assert_eq!(optional_decimal(&row, "spend"), Some(Decimal::ZERO));
    assert_eq!(optional_decimal(&row, "max_budget"), None);
    assert_eq!(json_column(&row, "aliases"), parsed_json("{}"));
    assert_eq!(json_column(&row, "config"), parsed_json("{}"));
    assert_eq!(json_column(&row, "permissions"), parsed_json("{}"));
    assert_eq!(json_column(&row, "metadata"), parsed_json("{}"));
    assert_eq!(json_column(&row, "model_spend"), parsed_json("{}"));
    assert_eq!(json_column(&row, "model_max_budget"), parsed_json("{}"));
    assert_eq!(
        row.get::<_, Option<Vec<String>>>("allowed_cache_controls"),
        Some(Vec::new())
    );
    assert!(row.get::<_, Option<NaiveDateTime>>("created_at").is_some());
    assert!(row.get::<_, Option<NaiveDateTime>>("updated_at").is_some());
}

#[tokio::test]
async fn postgres_failure_effects() {
    let postgres = RepairCase::provision().await;
    let data = csv_data(&[
        &[
            ("token", "tok-first"),
            ("key_name", "first"),
            ("max_parallel_requests", "1"),
        ],
        &[
            ("token", "tok-overflow"),
            ("key_name", "overflow"),
            ("max_parallel_requests", "5000000000"),
        ],
        &[
            ("token", "tok-last"),
            ("key_name", "last"),
            ("max_parallel_requests", "2"),
        ],
    ]);
    let report = import_csv(&postgres, &data).await;
    assert_eq!(report.migrated(), 2);
    assert_eq!(report.failed(), 1);
    assert_eq!(failed_token(&report).as_deref(), Some("tok-overflow"));
    assert!(postgres.token_present("tok-first").await);
    assert!(!postgres.token_present("tok-overflow").await);
    assert!(postgres.token_present("tok-last").await);
    assert_team_update_failure(&postgres).await;
    postgres.close().await;
}

fn failed_token(report: &ImportReport) -> Option<String> {
    report.events.iter().find_map(|event| match event {
        ImportEvent::Failed { token, source } => {
            assert!(matches!(source, ImportFailure::Insert(_)));
            token.clone()
        }
        ImportEvent::Migrated { .. } | ImportEvent::Skipped { .. } => None,
    })
}

async fn assert_team_update_failure(postgres: &RepairCase) {
    postgres.seed_spend_log("req-fail", "tok-first", None).await;
    postgres
        .client
        .batch_execute(r#"DROP TABLE "LiteLLM_VerificationToken""#)
        .await
        .unwrap();
    let error = apply_db_fixes(&postgres.client).await.unwrap_err();
    assert!(matches!(error, UnassignedTeamError::Update { .. }));
    assert_eq!(postgres.spend_team("req-fail").await, None);
}

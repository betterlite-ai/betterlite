#![cfg(test)]

use super::decoder::{
    TeamRecord, TeamRecordError, convert_record, decode_locked_roster, decode_team_record,
};
use super::repository::{TeamDatabase, TeamQueryError, TeamRepository};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::session::PostgresStore;
use crate::team_rows::decoder::{MAX_ROW_BYTES, TeamRowError, parse_data};
use crate::team_rows::member::{Member, MemberRole};
use crate::team_rows::team::LiteLLMTeamTable;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::ptr;
use std::str::FromStr as _;
use std::time::Duration;
use tokio::time::timeout;
use tokio_postgres::{Config, NoTls};

const STORAGE: &str = include_str!("storage.sql");
// the seed array plus these wraps is depth 65, one past the decoder's cap
const DEPTH_WRAPS: usize = 64;
const TEAM_ADVISORY_LOCK_SQL: &str = "SELECT pg_advisory_xact_lock(hashtext($1)) IS NULL AS locked";
const SET_ROSTER_SQL: &str =
    r#"UPDATE "LiteLLM_TeamTable" SET members_with_roles = $1::TEXT::JSONB WHERE team_id = $2"#;

fn parse(text: &str) -> JsonData {
    parse_data(text.as_bytes()).unwrap()
}

fn string(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

fn doubly(value: &JsonData) -> JsonData {
    JsonData::String(serde_json::to_string(value).unwrap())
}

// the repository decodes the stored text once and the model validator decodes it again
fn stored_twice(value: &JsonData) -> JsonData {
    doubly(&JsonData::String(serde_json::to_string(value).unwrap()))
}

fn object(fields: Vec<(&str, JsonData)>) -> BTreeMap<String, JsonData> {
    fields
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn record(text: &str) -> TeamRecord {
    TeamRecord::parse(text.as_bytes()).unwrap().unwrap()
}

fn decode(text: &str) -> Result<Option<LiteLLMTeamTable>, TeamRecordError> {
    let record = TeamRecord::parse(text.as_bytes())?;
    decode_team_record(record.as_ref())
}

fn decoded(fields: Vec<(&str, JsonData)>) -> Result<Option<LiteLLMTeamTable>, TeamRecordError> {
    let object = JsonData::Object(object(fields));
    let text = serde_json::to_string(&object).map_err(|source| TeamRecordError::Record {
        source: TeamRowError::Json(source),
    })?;
    decode(&text)
}

fn roster(value: &JsonData) -> Result<Vec<Member>, TeamRecordError> {
    decode_locked_roster(value)
}

fn member(text: &str) -> Member {
    Member::try_from(parse(text)).unwrap()
}

fn store(pool: &Pool) -> PostgresStore {
    PostgresStore {
        pool: Some(pool.clone()),
    }
}

fn repository(handle: &PostgresStore) -> TeamRepository<'_> {
    TeamRepository {
        database: TeamDatabase {
            db: handle,
            writer_db: handle,
        },
        use_writer: false,
    }
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let configured = Environment::parse_from(["team-repository-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(configured.as_ref())
        .resolve()
        .await
        .unwrap();
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(4)
        .build()
        .unwrap();
    (fixture, pool)
}

async fn seeded() -> (PostgresFixture, Pool) {
    let (fixture, pool) = database().await;
    pool.get()
        .await
        .unwrap()
        .batch_execute(STORAGE)
        .await
        .unwrap();
    (fixture, pool)
}

async fn finish(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.unwrap();
}

fn as_failure(result: &Result<Vec<Member>, TeamRecordError>) -> &'static str {
    match result {
        Err(TeamRecordError::NotAList) => "not a list",
        Err(TeamRecordError::Member { .. }) => "member",
        Err(
            TeamRecordError::Record { .. }
            | TeamRecordError::Field { .. }
            | TeamRecordError::NotAnObject,
        ) => "record",
        Ok(_) => "decoded",
    }
}

#[test]
fn absent_team_record() {
    assert!(decode_team_record(None).unwrap().is_none());
    assert!(TeamRecord::parse(b"null").unwrap().is_none());
    assert!(matches!(
        TeamRecord::parse(b"[]"),
        Err(TeamRecordError::NotAnObject)
    ));
    assert!(matches!(
        TeamRecord::parse(b"\"text\""),
        Err(TeamRecordError::NotAnObject)
    ));
}

fn assert_six_fields_decoded() {
    let team = decode(
        r#"{"team_id":"t","metadata":"{\"stored\":true}","model_spend":"{\"a\":1.5}","model_max_budget":"{\"b\":2.5}","router_settings":"{\"rpm\":10}","budget_limits":"[{\"budget_duration\":\"1d\",\"max_budget\":1.5}]","members_with_roles":"[{\"user_id\":\"u\",\"role\":\"admin\"}]"}"#,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        team.fields.base.metadata,
        Some(object(vec![("stored", JsonData::Bool(true))]))
    );
    assert_eq!(
        team.fields.model_spend.0,
        Some(object(vec![("a", parse("1.5"))]))
    );
    assert_eq!(
        team.fields.model_max_budget.0,
        Some(object(vec![("b", parse("2.5"))]))
    );
    assert_eq!(
        team.fields.base.router_settings,
        Some(object(vec![("rpm", parse("10"))]))
    );
    let windows = team.fields.base.budget_limits.unwrap();
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].budget_duration, "1d");
    assert_eq!(windows[0].max_budget, Decimal::from_str("1.5").unwrap());
    assert_eq!(
        team.fields.base.members_with_roles,
        [member(r#"{"user_id":"u","role":"admin"}"#)]
    );
}

#[test]
fn six_team_json_fields() {
    assert_six_fields_decoded();
    let already = decode(r#"{"team_id":"t","metadata":{"stored":true}}"#)
        .unwrap()
        .unwrap();
    assert_eq!(
        already.fields.base.metadata,
        Some(object(vec![("stored", JsonData::Bool(true))]))
    );
}

#[test]
fn malformed_json_fails_before_model_validation() {
    // team_id is missing, so the model would fail, but the stored JSON error comes first
    assert!(matches!(
        decode(r#"{"metadata":"{"}"#),
        Err(TeamRecordError::Field { field, .. }) if field == "metadata"
    ));
    assert!(matches!(
        decode(r#"{"metadata":"{\"a\":1}"}"#),
        Err(TeamRecordError::Record {
            source: TeamRowError::MissingTeamId
        })
    ));
}

#[test]
fn input_mapping_unchanged() {
    let original = record(r#"{"team_id":"t","metadata":"{\"a\":1}","members_with_roles":"[]"}"#);
    let before = original.clone();
    let team = convert_record(&original).unwrap();
    assert_eq!(original, before);
    assert_eq!(original.fields.get("metadata"), Some(&string("{\"a\":1}")));
    assert_eq!(
        team.fields.base.metadata,
        Some(object(vec![("a", parse("1"))]))
    );
}

#[test]
fn mapping_order_names_the_first_invalid_field() {
    assert!(matches!(
        decode(r#"{"team_id":"t","router_settings":"bad","metadata":"bad"}"#),
        Err(TeamRecordError::Field { field, .. }) if field == "router_settings"
    ));
}

#[test]
fn duplicate_key_in_stored_field_is_refused() {
    assert!(matches!(
        decode(r#"{"team_id":"t","metadata":"{\"a\":1,\"a\":2}"}"#),
        Err(TeamRecordError::Field { field, .. }) if field == "metadata"
    ));
}

fn assert_overlapping_fields_decode_twice() {
    let team = decoded(vec![
        ("team_id", string("t")),
        ("metadata", stored_twice(&parse(r#"{"a":1}"#))),
        ("model_max_budget", stored_twice(&parse(r#"{"b":2}"#))),
        ("router_settings", stored_twice(&parse(r#"{"rpm":3}"#))),
        (
            "budget_limits",
            stored_twice(&parse(r#"[{"budget_duration":"1d","max_budget":1.5}]"#)),
        ),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(
        team.fields.base.metadata,
        Some(object(vec![("a", parse("1"))]))
    );
    assert_eq!(
        team.fields.model_max_budget.0,
        Some(object(vec![("b", parse("2"))]))
    );
    assert_eq!(
        team.fields.base.router_settings,
        Some(object(vec![("rpm", parse("3"))]))
    );
    assert_eq!(
        team.fields.base.budget_limits.map(|windows| windows.len()),
        Some(1)
    );
}

fn assert_unlisted_fields_stay_text() {
    assert!(matches!(
        decoded(vec![
            ("team_id", string("t")),
            ("model_spend", stored_twice(&parse(r#"{"a":1}"#))),
        ]),
        Err(TeamRecordError::Record { .. })
    ));
    assert!(matches!(
        decoded(vec![
            ("team_id", string("t")),
            (
                "members_with_roles",
                stored_twice(&parse(r#"[{"user_id":"u","role":"admin"}]"#))
            ),
        ]),
        Err(TeamRecordError::Record { .. })
    ));
}

#[test]
fn second_model_parse_only_for_overlapping_fields() {
    assert_overlapping_fields_decode_twice();
    assert_unlisted_fields_stay_text();
}

#[test]
fn unlisted_model_extras_reach_validator() {
    let team = decoded(vec![
        ("team_id", string("t")),
        ("aliases", doubly(&parse(r#"{"a":1}"#))),
        ("config", doubly(&parse(r#"{"c":true}"#))),
        ("permissions", doubly(&parse("[]"))),
        ("model_aliases", doubly(&parse(r#"{"m":"x"}"#))),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(team.fields.base.team_id, "t");
    assert!(matches!(
        decoded(vec![("team_id", string("t")), ("aliases", string("{bad")),]),
        Err(TeamRecordError::Record {
            source: TeamRowError::Field {
                field: "aliases",
                ..
            }
        })
    ));
}

#[test]
fn full_team_null_vs_empty_dict_roster() {
    let emptied = decoded(vec![
        ("team_id", string("t")),
        ("members_with_roles", parse("{}")),
    ])
    .unwrap()
    .unwrap();
    assert!(emptied.fields.base.members_with_roles.is_empty());
    let text = decoded(vec![
        ("team_id", string("t")),
        ("members_with_roles", string("{}")),
    ])
    .unwrap()
    .unwrap();
    assert!(text.fields.base.members_with_roles.is_empty());
    assert!(matches!(
        decoded(vec![
            ("team_id", string("t")),
            ("members_with_roles", JsonData::Null),
        ]),
        Err(TeamRecordError::Record { .. })
    ));
    assert!(matches!(
        decoded(vec![
            ("team_id", string("t")),
            ("members_with_roles", string("null")),
        ]),
        Err(TeamRecordError::Record { .. })
    ));
}

#[rstest]
#[case::null("null")]
#[case::false_value("false")]
#[case::zero("0")]
#[case::zero_fraction("0.0")]
#[case::negative_zero("-0.0")]
#[case::empty_object("{}")]
#[case::empty_array("[]")]
#[case::decoded_empty_string("\"\\\"\\\"\"")]
#[case::decoded_empty_object("\"{}\"")]
fn locked_roster_all_falsey_values(#[case] text: &str) {
    assert!(roster(&parse(text)).unwrap().is_empty());
}

#[test]
fn raw_empty_string_and_malformed_json_fail() {
    // a raw empty string is not JSON, while a decoded empty string is a falsey roster
    assert!(matches!(
        roster(&parse("\"\"")),
        Err(TeamRecordError::Record { .. })
    ));
    assert!(matches!(
        roster(&parse("\"{\"")),
        Err(TeamRecordError::Record { .. })
    ));
}

fn assert_truthy_nonlist_rejected() {
    assert_eq!(as_failure(&roster(&parse("true"))), "not a list");
    assert_eq!(as_failure(&roster(&parse("1"))), "not a list");
    assert_eq!(
        as_failure(&roster(&parse(r#"{"user_id":"u"}"#))),
        "not a list"
    );
    assert_eq!(as_failure(&roster(&parse("\"\\\"x\\\"\""))), "not a list");
}

fn assert_invalid_members_rejected() {
    assert_eq!(
        as_failure(&roster(&parse(r#"[{"user_id":"u"}]"#))),
        "member"
    );
    assert_eq!(
        as_failure(&roster(&parse(r#"[{"user_id":"u","role":"owner"}]"#))),
        "member"
    );
    assert_eq!(
        as_failure(&roster(&parse(r#"[{"role":"admin"}]"#))),
        "member"
    );
    assert_eq!(as_failure(&roster(&parse(r#"["u"]"#))), "member");
}

#[test]
fn truthy_nonlist_and_invalid_member_fail() {
    assert_truthy_nonlist_rejected();
    assert_invalid_members_rejected();
}

#[test]
fn roster_order_and_duplicates_retained() {
    let members = roster(&parse(
        r#"[{"user_id":"b","role":"user"},{"user_id":"a","role":"admin"},{"user_id":"b","role":"user"}]"#,
    ))
    .unwrap();
    assert_eq!(members.len(), 3);
    assert_eq!(members[0].base.user_id.as_deref(), Some("b"));
    assert_eq!(members[1].base.user_id.as_deref(), Some("a"));
    assert_eq!(members[1].role, MemberRole::Admin);
    assert_eq!(members[0], members[2]);
}

#[test]
fn native_team_record_limits() {
    let prefix = r#"{"team_id":"t","pad":""#;
    let suffix = "\"}";
    let inside = format!(
        "{prefix}{}{suffix}",
        "x".repeat(MAX_ROW_BYTES - prefix.len() - suffix.len())
    );
    assert_eq!(inside.len(), MAX_ROW_BYTES);
    assert!(decode(&inside).unwrap().is_some());
    assert!(matches!(
        TeamRecord::parse(format!("{inside} ").as_bytes()),
        Err(TeamRecordError::Record {
            source: TeamRowError::TooLarge { .. }
        })
    ));
    let nested = |depth: usize| {
        format!(
            r#"{{"team_id":"t","pad":{}{}}}"#,
            "[".repeat(depth),
            "]".repeat(depth)
        )
    };
    assert!(decode(&nested(63)).is_ok(), "depth 63 fits under the cap");
    assert!(matches!(
        decode(&nested(64)),
        Err(TeamRecordError::Record {
            source: TeamRowError::TooDeep
        })
    ));
    // a stored string's own decode is bounded too, and its depth hides inside the record's quotes
    let deep = (0..DEPTH_WRAPS).fold(JsonData::Array(vec![parse("0")]), |inner, _| {
        JsonData::Array(vec![inner])
    });
    assert!(matches!(
        decoded(vec![("team_id", string("t")), ("metadata", doubly(&deep))]),
        Err(TeamRecordError::Field {
            field,
            source: TeamRowError::TooDeep
        }) if field == "metadata"
    ));
}

#[test]
fn native_locked_roster_limits() {
    // a roster column arrives as stored text, so every limit applies to the decoder's own parse
    let falsey_inside = format!("{}null", " ".repeat(MAX_ROW_BYTES - 4));
    assert_eq!(falsey_inside.len(), MAX_ROW_BYTES);
    assert!(roster(&string(&falsey_inside)).unwrap().is_empty());
    assert!(matches!(
        roster(&string(&format!("{falsey_inside} "))),
        Err(TeamRecordError::Record {
            source: TeamRowError::TooLarge { .. }
        })
    ));
    let nested = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
    // depth 64 still reaches member validation; only the entries themselves are not members
    assert_eq!(as_failure(&roster(&string(&nested(64)))), "member");
    assert!(matches!(
        roster(&string(&nested(65))),
        Err(TeamRecordError::Record {
            source: TeamRowError::TooDeep
        })
    ));
}

#[test]
fn default_db_and_explicit_writer() {
    let reader = PostgresStore { pool: None };
    let writer = PostgresStore { pool: None };
    let database = TeamDatabase {
        db: &reader,
        writer_db: &writer,
    };
    let default = TeamRepository {
        database,
        use_writer: false,
    };
    assert!(ptr::eq(default.table().store, &raw const reader));
    let pinned = TeamRepository {
        database,
        use_writer: true,
    };
    assert!(ptr::eq(pinned.table().store, &raw const writer));
}

#[tokio::test]
async fn team_id_default_and_missing_row() {
    let (fixture, pool) = seeded().await;
    let handle = store(&pool);
    let teams = repository(&handle);
    let found = teams.find_by_id("t").await.unwrap().unwrap();
    assert_eq!(found.fields.base.team_id, "t");
    assert_eq!(found.fields.base.team_alias.as_deref(), Some("alpha"));
    assert!(teams.find_by_id("absent").await.unwrap().is_none());
    finish(fixture, pool).await;
}

#[tokio::test]
async fn organization_id_exact_filter_and_all_rows() {
    let (fixture, pool) = seeded().await;
    let handle = store(&pool);
    let teams = repository(&handle);
    let found = teams.find_by_organization_id("org-a").await.unwrap();
    let ids = found
        .iter()
        .map(|team| team.fields.base.team_id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids, BTreeSet::from(["empty", "strings", "t"]));
    let missing = teams.find_by_organization_id("org-missing").await.unwrap();
    assert!(missing.is_empty());
    finish(fixture, pool).await;
}

#[tokio::test]
async fn raw_table_fields_bypass_conversion() {
    let (fixture, pool) = seeded().await;
    let handle = store(&pool);
    let teams = repository(&handle);
    let raw = teams.table().find_unique("strings").await.unwrap().unwrap();
    assert_eq!(
        raw.fields.get("metadata"),
        Some(&string("{\"stored\": true}"))
    );
    let converted = teams.find_by_id("strings").await.unwrap().unwrap();
    assert_eq!(
        converted.fields.base.metadata,
        Some(object(vec![("stored", JsonData::Bool(true))]))
    );
    finish(fixture, pool).await;
}

#[tokio::test]
async fn postgres_roster_explicit_transaction() {
    let (fixture, pool) = seeded().await;
    let mut client = pool.get().await.unwrap();
    let transaction = client.transaction().await.unwrap();
    transaction
        .execute(
            SET_ROSTER_SQL,
            &[&r#"[{"user_id":"u2","role":"user"}]"#, &"t"],
        )
        .await
        .unwrap();
    let locked = TeamRepository::get_members_with_roles_locked(&transaction, "t")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(locked[0].base.user_id.as_deref(), Some("u2"));
    transaction.rollback().await.unwrap();
    let mut reload = pool.get().await.unwrap();
    let reread = reload.transaction().await.unwrap();
    let original = TeamRepository::get_members_with_roles_locked(&reread, "t")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original[0].base.user_id.as_deref(), Some("u1"));
    reread.rollback().await.unwrap();
    finish(fixture, pool).await;
}

#[tokio::test]
async fn missing_team_none_existing_empty_team_list() {
    let (fixture, pool) = seeded().await;
    let mut client = pool.get().await.unwrap();
    let transaction = client.transaction().await.unwrap();
    assert_eq!(
        TeamRepository::get_members_with_roles_locked(&transaction, "absent")
            .await
            .unwrap(),
        None
    );
    let empty = TeamRepository::get_members_with_roles_locked(&transaction, "empty")
        .await
        .unwrap()
        .unwrap();
    assert!(empty.is_empty());
    transaction.rollback().await.unwrap();
    finish(fixture, pool).await;
}

#[tokio::test]
async fn postgres_caller_advisory_lock_serializes_reread() {
    let (fixture, pool) = seeded().await;
    let mut holder = pool.get().await.unwrap();
    let holding = holder.transaction().await.unwrap();
    holding
        .query_one(TEAM_ADVISORY_LOCK_SQL, &[&"t"])
        .await
        .unwrap();
    holding
        .execute(
            SET_ROSTER_SQL,
            &[&r#"[{"user_id":"u3","role":"admin"}]"#, &"t"],
        )
        .await
        .unwrap();
    let mut waiter = pool.get().await.unwrap();
    let waiting = waiter.transaction().await.unwrap();
    let mut acquisition = Box::pin(waiting.query_one(TEAM_ADVISORY_LOCK_SQL, &[&"t"]));
    let blocked = timeout(Duration::from_millis(200), &mut acquisition).await;
    assert!(
        blocked.is_err(),
        "the waiter took the lock while the holder still held it"
    );
    holding.commit().await.unwrap();
    acquisition.await.unwrap();
    let reread = TeamRepository::get_members_with_roles_locked(&waiting, "t")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reread[0].base.user_id.as_deref(), Some("u3"));
    waiting.rollback().await.unwrap();
    finish(fixture, pool).await;
}

#[tokio::test]
async fn postgres_plain_roster_select_avoids_row_lock() {
    let (fixture, pool) = seeded().await;
    let mut holder = pool.get().await.unwrap();
    let holding = holder.transaction().await.unwrap();
    let locked = holding
        .query_one(
            r#"SELECT 1 FROM "LiteLLM_TeamTable" WHERE team_id = 't' FOR UPDATE"#,
            &[],
        )
        .await
        .unwrap();
    assert_eq!(locked.get::<_, i32>(0), 1_i32);
    let mut reader = pool.get().await.unwrap();
    let reading = reader.transaction().await.unwrap();
    let roster = timeout(
        Duration::from_secs(2),
        TeamRepository::get_members_with_roles_locked(&reading, "t"),
    )
    .await
    .expect("a plain select must not wait for the row lock")
    .unwrap()
    .unwrap();
    assert_eq!(roster[0].base.user_id.as_deref(), Some("u1"));
    reading.rollback().await.unwrap();
    holding.rollback().await.unwrap();
    finish(fixture, pool).await;
}

async fn seed_error_table(pool: &Pool) {
    pool.get()
        .await
        .unwrap()
        .batch_execute(
            r#"CREATE TABLE "LiteLLM_TeamTable" (team_id TEXT PRIMARY KEY, members_with_roles JSONB NOT NULL DEFAULT '{}')"#,
        )
        .await
        .unwrap();
    pool.get()
        .await
        .unwrap()
        .batch_execute(
            r#"INSERT INTO "LiteLLM_TeamTable" (team_id, members_with_roles) VALUES ('broken', '"not json"'::JSONB), ('kept', '{"u1": "admin"}'::JSONB)"#,
        )
        .await
        .unwrap();
}

async fn assert_rows_unchanged(pool: &Pool) {
    let mut client = pool.get().await.unwrap();
    let transaction = client.transaction().await.unwrap();
    let rows = transaction
        .query(
            r#"SELECT team_id, members_with_roles::TEXT FROM "LiteLLM_TeamTable" ORDER BY team_id"#,
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<_, String>(0), "broken");
    assert_eq!(rows[0].get::<_, String>(1), "\"not json\"");
    assert_eq!(rows[1].get::<_, String>(0), "kept");
    assert_eq!(rows[1].get::<_, String>(1), "{\"u1\": \"admin\"}");
    transaction.rollback().await.unwrap();
}

#[tokio::test]
async fn query_error_does_not_write() {
    let (fixture, pool) = database().await;
    seed_error_table(&pool).await;
    let handle = store(&pool);
    let query_failure = repository(&handle)
        .find_by_organization_id("org")
        .await
        .unwrap_err();
    assert!(matches!(query_failure, TeamQueryError::Database { .. }));
    let mut client = pool.get().await.unwrap();
    let transaction = client.transaction().await.unwrap();
    let invalid = TeamRepository::get_members_with_roles_locked(&transaction, "broken")
        .await
        .unwrap_err();
    assert!(matches!(invalid, TeamQueryError::Record { .. }));
    transaction.rollback().await.unwrap();
    assert_rows_unchanged(&pool).await;
    finish(fixture, pool).await;
}

#[rstest]
fn replays_the_team_record_corpus(#[files("fuzz/corpus/team_record/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    let team = TeamRecord::parse(&bytes).and_then(|record| decode_team_record(record.as_ref()));
    let roster = parse_data(&bytes)
        .map_err(|source| TeamRecordError::Record { source })
        .and_then(|value| decode_locked_roster(&value));
    if name.starts_with("valid-record-") {
        assert!(team.unwrap().is_some(), "{}", path.display());
    } else if name.starts_with("invalid-record-") {
        assert!(team.is_err(), "{}", path.display());
    } else if name.starts_with("valid-roster-") {
        assert!(roster.is_ok(), "{}", path.display());
    } else {
        assert!(roster.is_err(), "{}", path.display());
    }
}

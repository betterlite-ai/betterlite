#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::schema_baseline;
use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tracing::Subscriber;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::sso_types::identity::OpenIdIdentity;

use super::build::build_attribution_metadata;
use super::claim_map::{CliClaim, decode_cli_claim_map, is_safe_claim_value, is_safe_destination};
use super::key_path::split_key_path;
use super::metadata::{flatten_metadata_for_poll, merge_metadata, poll_attribution_metadata};
use super::persist::{PersistError, UserMetadataRow, persist_attribution_metadata};

const BASE_IDENTITY: &str = r#"{"id": "cli-user", "email": "cli@example.com", "first_name": null,
    "last_name": null, "display_name": null, "picture": "https://img.example/p.png",
    "provider": "generic", "team_ids": [], "user_role": "proxy_admin", "extra_fields": null}"#;

const INSERT_USER_WITH_METADATA: &str =
    r#"INSERT INTO "LiteLLM_UserTable" (user_id, metadata) VALUES ($1, $2::text::jsonb)"#;
const SELECT_USER_METADATA: &str =
    r#"SELECT metadata::text AS metadata FROM "LiteLLM_UserTable" WHERE user_id = $1"#;
const COUNT_USERS: &str = r#"SELECT count(*) FROM "LiteLLM_UserTable" WHERE user_id = $1"#;

fn document(text: &str) -> BTreeMap<String, JsonData> {
    serde_json::from_str(text).unwrap()
}

fn open_id(extra_fields: &str) -> OpenIdIdentity {
    let mut identity: OpenIdIdentity = serde_json::from_str(BASE_IDENTITY).unwrap();
    identity.extra_fields = serde_json::from_str(extra_fields).unwrap();
    identity
}

fn builds(extra_fields: &str, claim_map: &str) -> BTreeMap<String, JsonData> {
    build_attribution_metadata(&open_id(extra_fields), &decode_cli_claim_map(claim_map)).unwrap()
}

fn entry(claim: &CliClaim) -> String {
    format!("{}->{}", claim.source, claim.destination)
}

#[rstest]
#[case::empty("", &[])]
#[case::whitespace("   ", &[])]
#[case::single("employment_type->acme_employment_type", &["employment_type->acme_employment_type"])]
#[case::spaces(" org_info.department -> department , label -> name ", &["org_info.department->department", "label->name"])]
#[case::metadata_prefix(
    "a->metadata.department,b->metadata.metadata.deep",
    &["a->department", "b->metadata.deep"]
)]
#[case::escaped_dot(r"org_info\.department->flat_department", &[r"org_info\.department->flat_department"])]
#[case::missing_arrow("no-arrow", &[])]
#[case::empty_sides("->", &[])]
#[case::empty_destination("label->", &[])]
#[case::empty_source("->name", &[])]
#[case::prefix_only_destination("label->metadata.", &[])]
#[case::source_keeps_metadata_prefix("metadata.->name", &["metadata.->name"])]
#[case::multiple_arrows("a->b->c", &["a->b->c"])]
#[case::trailing_comma("a->b,", &["a->b"])]
#[case::blank_entry("a->b,,c->d", &["a->b", "c->d"])]
fn decode_cli_claim_map_keeps_well_formed_entries(#[case] text: &str, #[case] expected: &[&str]) {
    let decoded = decode_cli_claim_map(text)
        .iter()
        .map(entry)
        .collect::<Vec<String>>();
    let wanted = expected
        .iter()
        .map(|claim| (*claim).to_owned())
        .collect::<Vec<String>>();
    assert_eq!(decoded, wanted);
}

#[rstest]
#[case::plain("acme_employment_type", true)]
#[case::dots_and_dashes("profile.department-name", true)]
#[case::empty("", false)]
#[case::space("profile department", false)]
#[case::slash("profile/department", false)]
#[case::access_token("access_token", false)]
#[case::mixed_case_secret("Profile.Api_Key", false)]
#[case::password("user_password_hash", false)]
#[case::secret("secret_santa", false)]
#[case::client_secret("client_secret_value", false)]
#[case::id_token("id_token", false)]
#[case::refresh_token("refresh_token", false)]
#[case::private_key("private_key", false)]
#[case::token_without_fragment("token", true)]
fn is_safe_destination_rejects_secret_names(#[case] destination: &str, #[case] expected: bool) {
    assert_eq!(is_safe_destination(destination), expected);
}

#[test]
fn a_destination_must_be_ascii() {
    // 233 is Latin-1 for the accented e, so the destination is not ASCII
    let accented = char::from(233_u8);
    assert!(!is_safe_destination(&format!("profil{accented}")));
}

#[rstest]
#[case::string(r#""value""#, true)]
#[case::integer("42", true)]
#[case::float("1.5", true)]
#[case::boolean("true", true)]
#[case::null("null", false)]
#[case::array("[1]", false)]
#[case::object(r#"{"a": 1}"#, false)]
#[case::token_with_two_dots(r#""eyJhbGci.eyJzdWIi.signature""#, false)]
#[case::token_with_one_dot(r#""eyJhbGci.eyJzdWIi""#, true)]
#[case::token_header_only(r#""eyJhbGciOiJIUzI1NiJ9""#, true)]
#[case::token_prefix_elsewhere(r#""xeyJ.a.b""#, true)]
fn is_safe_claim_value_keeps_only_plain_scalars(#[case] text: &str, #[case] expected: bool) {
    assert_eq!(
        is_safe_claim_value(&serde_json::from_str(text).unwrap()),
        expected
    );
}

#[test]
fn a_claim_string_stops_at_1024_characters() {
    // 233 is Latin-1 for the accented e, so each character is two bytes in UTF-8
    let accented = char::from(233_u8).to_string();
    assert!(is_safe_claim_value(&JsonData::String("a".repeat(1024))));
    assert!(!is_safe_claim_value(&JsonData::String("a".repeat(1025))));
    assert!(is_safe_claim_value(&JsonData::String(
        accented.repeat(1024)
    )));
    assert!(!is_safe_claim_value(&JsonData::String(
        accented.repeat(1025)
    )));
}

#[rstest]
#[case::empty("", &[""])]
#[case::plain("a", &["a"])]
#[case::two_parts("a.b", &["a", "b"])]
#[case::trailing_dot("a.", &["a", ""])]
#[case::leading_dot(".a", &["", "a"])]
#[case::escaped_dot(r"a\.b", &["a.b"])]
#[case::escaped_dot_in_path(r"a\.b.c", &["a.b", "c"])]
#[case::escaped_dot_then_dot(r"a\..b", &["a.", "b"])]
#[case::nul_reads_as_a_dot("a\u{0}b.c", &["a.b", "c"])]
#[case::lone_backslash(r"a\b", &[r"a\b"])]
#[case::double_backslash(r"a\\b", &[r"a\\b"])]
fn split_key_path_honours_escaped_dots(#[case] key_path: &str, #[case] expected: &[&str]) {
    assert_eq!(split_key_path(key_path), expected);
}

#[test]
fn build_keeps_nesting_and_scalar_types() {
    let metadata = builds(
        r#"{"count": 42, "numeric_text": "42", "ratio": 1.5, "enabled": true}"#,
        "count->profile.count,numeric_text->profile.numeric_text,ratio->profile.ratio,enabled->profile.enabled,missing->profile.missing",
    );
    assert_eq!(
        metadata,
        document(
            r#"{"profile": {"count": 42, "numeric_text": "42", "ratio": 1.5, "enabled": true}}"#
        )
    );
}

#[test]
fn build_prefers_a_literal_claim_key_before_traversal() {
    let metadata = builds(
        r#"{"org_info.department": "flat-value", "org_info": {"department": "nested-value", "team": "nested-team"}}"#,
        r"org_info.department->department,org_info\.department->flat_department,org_info.team->team",
    );
    assert_eq!(
        metadata,
        document(
            r#"{"department": "flat-value", "flat_department": "flat-value", "team": "nested-team"}"#
        )
    );
}

#[test]
fn build_prefers_extra_fields_before_the_identity_document() {
    let claim_map = "email->email,picture->profile_picture";
    let from_extra = builds(
        r#"{"email": "extra@example.com", "picture": null}"#,
        claim_map,
    );
    assert_eq!(from_extra, document(r#"{"email": "extra@example.com"}"#));
    let from_identity = builds(r#"{}"#, claim_map);
    assert_eq!(
        from_identity,
        document(r#"{"email": "cli@example.com", "profile_picture": "https://img.example/p.png"}"#)
    );
    let without_extra_fields =
        build_attribution_metadata(&open_id("null"), &decode_cli_claim_map(claim_map)).unwrap();
    assert_eq!(without_extra_fields, from_identity);
}

#[test]
fn build_skips_unsafe_destinations_and_values() {
    let metadata = builds(
        r#"{"jwt_claim": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.signature", "plain": "eyJhbGciOiJIUzI1NiJ9", "safe": "value", "note": "shown"}"#,
        "jwt_claim->token_note,plain->token_shape_ok,safe->api_key,note->secret_santa",
    );
    assert_eq!(
        metadata,
        document(r#"{"token_shape_ok": "eyJhbGciOiJIUzI1NiJ9"}"#)
    );
}

#[test]
fn build_replaces_a_scalar_parent_with_nested_metadata() {
    let nested_last = builds(
        r#"{"label": "acme", "region": "eu"}"#,
        "label->profile,region->profile.region",
    );
    assert_eq!(nested_last, document(r#"{"profile": {"region": "eu"}}"#));
    let scalar_last = builds(
        r#"{"label": "acme", "region": "eu"}"#,
        "region->profile.region,label->profile",
    );
    assert_eq!(scalar_last, document(r#"{"profile": "acme"}"#));
}

#[test]
fn build_of_an_empty_claim_map_is_empty() {
    assert!(builds(r#"{"employment_type": "full_time"}"#, "").is_empty());
    assert!(builds(r#"{"employment_type": "full_time"}"#, "  ").is_empty());
}

#[test]
fn build_reads_the_top_level_identity_fields() {
    let metadata = builds("{}", "team_ids->teams,provider->idp,user_role->role");
    assert_eq!(
        metadata,
        document(r#"{"idp": "generic", "role": "proxy_admin"}"#)
    );
    let with_extra_email = build_attribution_metadata(
        &open_id(r#"{"email": "extra@example.com"}"#),
        &decode_cli_claim_map("provider->idp,email->email"),
    )
    .unwrap();
    assert_eq!(
        with_extra_email,
        document(r#"{"idp": "generic", "email": "extra@example.com"}"#)
    );
}

#[test]
fn merge_keeps_unrelated_siblings_and_string_values() {
    let mut existing =
        document(r#"{"scim_active": false, "profile": {"region": "us", "tier": 2}}"#);
    merge_metadata(
        &mut existing,
        &document(r#"{"profile": {"region": "eu"}, "employment_type": "full_time"}"#),
    );
    assert_eq!(
        existing,
        document(
            r#"{"scim_active": false, "profile": {"region": "eu", "tier": 2}, "employment_type": "full_time"}"#
        )
    );
}

#[test]
fn merge_skips_null_values_and_replaces_other_shapes() {
    let mut existing = document(r#"{"profile": {"tier": 2}, "count": 7, "note": "keep"}"#);
    merge_metadata(
        &mut existing,
        &document(
            r#"{"count": "42", "note": null, "profile": {"tier": null, "region": "eu"}, "flag": false}"#,
        ),
    );
    assert_eq!(
        existing,
        document(
            r#"{"profile": {"tier": 2, "region": "eu"}, "count": "42", "note": "keep", "flag": false}"#
        )
    );
}

#[test]
fn merge_replaces_both_directions_between_scalars_and_objects() {
    let mut object_over_scalar = document(r#"{"profile": "scalar"}"#);
    merge_metadata(
        &mut object_over_scalar,
        &document(r#"{"profile": {"region": "eu"}}"#),
    );
    assert_eq!(
        object_over_scalar,
        document(r#"{"profile": {"region": "eu"}}"#)
    );
    let mut scalar_over_object = document(r#"{"profile": {"region": "eu"}}"#);
    merge_metadata(&mut scalar_over_object, &document(r#"{"profile": 5}"#));
    assert_eq!(scalar_over_object, document(r#"{"profile": 5}"#));
}

#[test]
fn merge_writes_an_empty_object_over_a_scalar_parent() {
    let mut existing = document(r#"{"profile": "scalar", "kept": 1}"#);
    merge_metadata(&mut existing, &document(r#"{"profile": {}}"#));
    assert_eq!(existing, document(r#"{"profile": {}, "kept": 1}"#));
    let mut object_parent = document(r#"{"profile": {"tier": 2}}"#);
    merge_metadata(&mut object_parent, &document(r#"{"profile": {}}"#));
    assert_eq!(object_parent, document(r#"{"profile": {"tier": 2}}"#));
}

#[test]
fn merge_into_an_empty_target_copies_the_source() {
    let mut existing = BTreeMap::new();
    let source = document(r#"{"profile": {"region": "eu"}, "employment_type": "full_time"}"#);
    merge_metadata(&mut existing, &source);
    assert_eq!(existing, source);
}

#[test]
fn flatten_exposes_safe_scalars_with_dotted_keys() {
    let flattened = flatten_metadata_for_poll(&document(
        r#"{"a": 1, "b": {"c": true, "d": {"e": "value"}}, "f.g": {"h": 1.5}}"#,
    ));
    assert_eq!(
        flattened,
        document(r#"{"a": 1, "b.c": true, "b.d.e": "value", "f.g.h": 1.5}"#)
    );
}

#[test]
fn flatten_drops_unsafe_values_and_containers() {
    let token = "eyJhbGci.eyJzdWIi.signature".to_owned();
    let long = "a".repeat(1025);
    let metadata = document(&format!(
        r#"{{"token": "{token}", "long": "{long}", "items": [1], "nothing": null, "empty": {{}}}}"#
    ));
    assert!(flatten_metadata_for_poll(&metadata).is_empty());
}

#[test]
fn poll_projects_the_stored_attribution_metadata() {
    let session = document(r#"{"attribution_metadata": {"a": 1, "b": {"c": true}}}"#);
    assert_eq!(
        poll_attribution_metadata(&session),
        document(r#"{"a": 1, "b.c": true}"#)
    );
    assert!(poll_attribution_metadata(&document("{}")).is_empty());
    assert!(poll_attribution_metadata(&document(r#"{"attribution_metadata": "text"}"#)).is_empty());
    assert!(poll_attribution_metadata(&document(r#"{"attribution_metadata": null}"#)).is_empty());
}

#[rstest]
#[case::object(r#"{"scim_active": false}"#, Some(r#"{"scim_active": false}"#))]
#[case::empty_object("{}", Some("{}"))]
#[case::null("null", None)]
#[case::array("[1, 2]", None)]
#[case::scalar("true", None)]
#[case::json_text_object(r#""{\"scim_active\": false}""#, Some(r#"{"scim_active": false}"#))]
#[case::json_text_array(r#""[1, 2]""#, None)]
#[case::json_text_broken(r#""not-json""#, None)]
fn user_metadata_row_reads_only_objects(#[case] text: &str, #[case] expected: Option<&str>) {
    let row = UserMetadataRow::decode(text).unwrap();
    assert_eq!(row.metadata, expected.map(document));
}

#[test]
fn user_metadata_row_reports_unreadable_column_text() {
    assert!(matches!(
        UserMetadataRow::decode("not-json"),
        Err(PersistError::Decode(_))
    ));
}

struct ErrorCounter(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for ErrorCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::ERROR {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

struct Errors {
    count: Arc<AtomicUsize>,
    _guard: DefaultGuard,
}

impl Errors {
    fn capture() -> Self {
        let count = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry().with(ErrorCounter(Arc::clone(&count)));
        Self {
            count,
            _guard: set_default(subscriber),
        }
    }

    fn count(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }
}

#[tokio::test]
async fn persist_without_attribution_never_reaches_the_database() {
    let errors = Errors::capture();
    persist_attribution_metadata(&PostgresStore { pool: None }, "cli-user", &BTreeMap::new()).await;
    assert_eq!(errors.count(), 0);
}

#[tokio::test]
async fn persist_logs_and_continues_when_the_database_is_unavailable() {
    let errors = Errors::capture();
    persist_attribution_metadata(
        &PostgresStore { pool: None },
        "cli-user",
        &document(r#"{"employment_type": "full_time"}"#),
    )
    .await;
    assert_eq!(errors.count(), 1);
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, PostgresStore) {
    let url = Environment::parse_from(["cli-attribution-tests"])
        .url
        .map(SecretString::from);
    let mut generator = RandomDatabaseName;
    let mut task = PostgresFixture::provision_with_name_generator(
        url.as_ref(),
        &FixtureSeed::default(),
        &mut generator,
    );
    let fixture = task.resolve().await.expect("the database fixture");
    let pool = Pool::builder(Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the database url"),
        NoTls,
    ))
    .max_size(2)
    .build()
    .expect("the database pool");
    let postgres = PostgresStore { pool: Some(pool) };
    let client = postgres.session().await.expect("the database connection");
    client
        .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .await
        .expect("the empty public schema");
    for migration in schema_baseline::MIGRATION_CATALOG.files {
        let sql = str::from_utf8(migration.bytes).expect("the migration text");
        client
            .batch_execute(sql)
            .await
            .expect("the schema migration");
    }
    (fixture, postgres)
}

async fn close(fixture: PostgresFixture, postgres: PostgresStore) {
    if let Some(pool) = postgres.pool {
        pool.close();
    }
    fixture.close_async().await.expect("the fixture teardown");
}

async fn insert_user(postgres: &PostgresStore, user_id: &str, metadata: &str) {
    let client = postgres.session().await.expect("the database connection");
    client
        .execute(INSERT_USER_WITH_METADATA, &[&user_id, &metadata])
        .await
        .expect("the user row");
}

async fn stored_metadata(postgres: &PostgresStore, user_id: &str) -> String {
    let client = postgres.session().await.expect("the database connection");
    let row = client
        .query_one(SELECT_USER_METADATA, &[&user_id])
        .await
        .expect("the stored user");
    row.try_get("metadata").expect("the metadata text")
}

async fn user_count(postgres: &PostgresStore, user_id: &str) -> i64 {
    let client = postgres.session().await.expect("the database connection");
    let row = client
        .query_one(COUNT_USERS, &[&user_id])
        .await
        .expect("the user count");
    row.try_get(0).expect("the count")
}

#[tokio::test]
async fn persist_merges_attribution_into_the_stored_user_metadata() {
    let (fixture, postgres) = database().await;
    insert_user(
        &postgres,
        "cli-user",
        r#"{"scim_active": false, "profile": {"region": "eu", "tier": 2}}"#,
    )
    .await;
    let attribution = builds(
        r#"{"employment_type": "full_time", "region": "us"}"#,
        "employment_type->acme_employment_type,region->profile.region",
    );
    persist_attribution_metadata(&postgres, "cli-user", &attribution).await;
    assert_eq!(
        document(&stored_metadata(&postgres, "cli-user").await),
        document(
            r#"{"scim_active": false, "profile": {"region": "us", "tier": 2}, "acme_employment_type": "full_time"}"#
        )
    );
    close(fixture, postgres).await;
}

#[tokio::test]
async fn persist_replaces_metadata_that_is_not_an_object() {
    let (fixture, postgres) = database().await;
    insert_user(&postgres, "text-user", r#""{\"scim_active\": false}""#).await;
    insert_user(&postgres, "array-user", "[1, 2]").await;
    insert_user(&postgres, "null-user", "null").await;
    insert_user(&postgres, "string-user", r#""plain""#).await;
    let attribution = document(r#"{"employment_type": "full_time"}"#);
    persist_attribution_metadata(&postgres, "text-user", &attribution).await;
    persist_attribution_metadata(&postgres, "array-user", &attribution).await;
    persist_attribution_metadata(&postgres, "null-user", &attribution).await;
    persist_attribution_metadata(&postgres, "string-user", &attribution).await;
    assert_eq!(
        document(&stored_metadata(&postgres, "text-user").await),
        document(r#"{"scim_active": false, "employment_type": "full_time"}"#)
    );
    assert_eq!(
        document(&stored_metadata(&postgres, "array-user").await),
        attribution
    );
    assert_eq!(
        document(&stored_metadata(&postgres, "null-user").await),
        attribution
    );
    assert_eq!(
        document(&stored_metadata(&postgres, "string-user").await),
        attribution
    );
    close(fixture, postgres).await;
}

#[tokio::test]
async fn persist_does_not_create_a_missing_user() {
    let (fixture, postgres) = database().await;
    persist_attribution_metadata(
        &postgres,
        "missing-user",
        &document(r#"{"employment_type": "full_time"}"#),
    )
    .await;
    assert_eq!(user_count(&postgres, "missing-user").await, 0);
    close(fixture, postgres).await;
}

#[rstest]
fn replays_the_cli_claim_map_corpus(#[files("fuzz/corpus/cli_claim_map/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    let claim_map = decode_cli_claim_map(text);
    assert!(
        claim_map
            .iter()
            .all(|claim| !claim.source.is_empty() && !claim.destination.is_empty())
    );
}

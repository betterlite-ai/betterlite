#![cfg(test)]

use super::email::{EMAIL_PAGE_SIZE, email_pages};
use super::placeholders::PlaceholderError;
use super::projection::{UserRecordError, decode_user_record, user_record_to_model};
use super::tables::{ORGANIZATION_TABLE, PROJECT_TABLE, USER_TABLE};
use super::{OrganizationRepository, ProjectRepository, RowDecodeError, UserRepository};
use crate::budget_permission_rows::JsonData;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::postgres_store::errors::DbError;
use crate::postgres_store::repository::Repository;
use crate::postgres_store::rows::KeyColumn;
use crate::resource_rows::ResourceRowsError;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, GenericClient, Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::timeout;
use tokio_postgres::NoTls;

const FIELD_BYTES: usize = 0x0010_0000;
const FIELD_DEPTH: usize = 64;

const READER_SEED: &str = r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_alias)
    VALUES ('id-value', 'alias-value'), ('second-row', NULL)"#;
const WRITER_SEED: &str = r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('writer-only')"#;

const ORGANIZATION_PROJECT_SEED: &str = r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by)
    VALUES ('budget-1', 'creator', 'editor'), ('budget-2', 'creator', 'editor');
    INSERT INTO "LiteLLM_OrganizationTable"
        (organization_id, organization_alias, budget_id, models, created_by, updated_by)
    VALUES ('org-1', 'alias-1', 'budget-1', ARRAY[]::TEXT[], 'creator', 'editor'),
           ('org-2', 'alias-2', 'budget-2', ARRAY[]::TEXT[], 'creator', 'editor');
    CREATE TABLE "LiteLLM_ProjectTable" (
        project_id TEXT PRIMARY KEY,
        project_alias TEXT,
        description TEXT,
        team_id TEXT,
        budget_id TEXT,
        metadata JSONB NOT NULL DEFAULT '{}',
        models TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
        spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
        model_spend JSONB NOT NULL DEFAULT '{}',
        model_rpm_limit JSONB NOT NULL DEFAULT '{}',
        model_tpm_limit JSONB NOT NULL DEFAULT '{}',
        blocked BOOLEAN NOT NULL DEFAULT false,
        object_permission_id TEXT,
        created_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
        created_by TEXT NOT NULL,
        updated_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_by TEXT NOT NULL
    );
    INSERT INTO "LiteLLM_ProjectTable" (project_id, project_alias, created_by, updated_by)
    VALUES ('project-1', 'project-alias-1', 'creator', 'editor'),
           ('project-2', 'project-alias-2', 'creator', 'editor')"#;

const EMAIL_MATCH_SEED: &str = r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES
    ('alice', 'Alice@Example.com'),
    ('bob', 'bob@example.com'),
    ('underscore', 'under_score@example.com'),
    ('underx', 'underXscore@example.com'),
    ('percent', 'percent%x@example.com'),
    ('percentx', 'percentXx@example.com')"#;

const EMAIL_LATER_PAGE_SEED: &str = r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email)
    VALUES ('first', 'first@x.com');
    INSERT INTO "LiteLLM_UserTable" (user_id, user_email, metadata)
    VALUES ('broken', 'z-last@x.com', '"not-json"'::jsonb)"#;

const PLACEHOLDER_PREDICATES_SEED: &str = r#"INSERT INTO "LiteLLM_UserTable" (user_id, sso_user_id, user_email, teams) VALUES
    ('placeholder-1', NULL, 'p1@example.com', ARRAY['t1']),
    ('placeholder-2', NULL, 'irrelevant@example.com', ARRAY[]::TEXT[]),
    ('placeholder-3', NULL, NULL, ARRAY[]::TEXT[]),
    ('placeholder-4', '', NULL, ARRAY[]::TEXT[]),
    ('placeholder-5', NULL, NULL, ARRAY[]::TEXT[]),
    ('placeholder-6', NULL, NULL, ARRAY[]::TEXT[]),
    ('placeholder-7', NULL, NULL, ARRAY[]::TEXT[]),
    ('resolver-1', 'placeholder-1', NULL, ARRAY[]::TEXT[]),
    ('resolver-2', NULL, 'PLACEHOLDER-2', ARRAY[]::TEXT[]),
    ('resolver-4', 'placeholder-4', NULL, ARRAY[]::TEXT[]);
    INSERT INTO "LiteLLM_VerificationToken" (token, user_id, blocked, expires) VALUES
    ('key-5', 'placeholder-5', false, NULL),
    ('key-6', 'placeholder-6', true, NULL),
    ('key-7', 'placeholder-7', false, '2020-01-01T00:00:00')"#;

const PLACEHOLDER_OR_SEED: &str = r#"INSERT INTO "LiteLLM_UserTable" (user_id, sso_user_id, user_email) VALUES
    ('placeholder-both', NULL, NULL),
    ('resolver-both', 'placeholder-both', 'Placeholder-Both'),
    ('resolver-email', NULL, 'placeholder-both')"#;

const PLACEHOLDER_ORDER_SEED: &str = r#"INSERT INTO "LiteLLM_UserTable" (user_id, teams) VALUES
    ('placeholder-z', ARRAY['t2', 't1', 't2']),
    ('placeholder-a', ARRAY['t0']),
    ('resolver-z-alpha', NULL),
    ('resolver-z-bravo', NULL),
    ('resolver-a', NULL);
    UPDATE "LiteLLM_UserTable" SET sso_user_id = 'placeholder-z'
        WHERE user_id = 'resolver-z-alpha';
    UPDATE "LiteLLM_UserTable" SET user_email = 'Placeholder-Z'
        WHERE user_id = 'resolver-z-bravo';
    UPDATE "LiteLLM_UserTable" SET sso_user_id = 'placeholder-a'
        WHERE user_id = 'resolver-a'"#;

const PLACEHOLDER_NULL_TEAMS_SEED: &str = r#"INSERT INTO "LiteLLM_UserTable" (user_id, teams) VALUES
    ('placeholder-a', ARRAY['t0']),
    ('placeholder-b', NULL);
    INSERT INTO "LiteLLM_UserTable" (user_id, sso_user_id) VALUES
    ('resolver-a', 'placeholder-a'),
    ('resolver-b', 'placeholder-b')"#;

const BILLABLE_SEED: &str = r#"ALTER TABLE "LiteLLM_UserTable" ALTER COLUMN metadata DROP NOT NULL;
    INSERT INTO "LiteLLM_UserTable" (user_id, metadata) VALUES
    ('missing', '{}'),
    ('sql-null', NULL),
    ('json-null', '{"scim_active": null}'),
    ('true', '{"scim_active": true}'),
    ('string-false', '{"scim_active": "false"}'),
    ('zero', '{"scim_active": 0}'),
    ('encoded', to_jsonb('{"scim_active": false}'::text)),
    ('deactivated-a', '{"scim_active": false}'),
    ('deactivated-b', '{"scim_active": false}')"#;

const RACE_SEED: &str =
    r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES ('race', NULL)"#;

const TWO_READS_SEED: &str = r#"INSERT INTO "LiteLLM_UserTable" (user_id, metadata) VALUES ('active', '{}');
    ALTER TABLE "LiteLLM_UserTable" RENAME TO "LiteLLM_UserTable_backing";
    CREATE FUNCTION b723_bump() RETURNS boolean LANGUAGE plpgsql AS $$
    BEGIN
      IF current_setting('b723.bumped', true) IS NULL THEN
        PERFORM set_config('b723.bumped', '1', false);
        INSERT INTO "LiteLLM_UserTable_backing" (user_id, metadata) VALUES
          ('late-a', '{"scim_active": false}'::jsonb),
          ('late-b', '{"scim_active": false}'::jsonb);
      END IF;
      RETURN true;
    END $$;
    CREATE VIEW "LiteLLM_UserTable" AS
      SELECT * FROM "LiteLLM_UserTable_backing" WHERE b723_bump()"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Database {
    fixture: PostgresFixture,
    pool: Pool,
}

impl Database {
    async fn start() -> Self {
        let url = Environment::parse_from(["user-repository-tests"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_schema(url.as_ref())
            .resolve()
            .await
            .unwrap();
        let config: tokio_postgres::Config = fixture.options.url.expose_secret().parse().unwrap();
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(4)
            .build()
            .unwrap();
        let client = pool.get().await.unwrap();
        client
            .batch_execute(include_str!(
                "../../../../tools/postgres-fixture/fixtures/baseline.sql"
            ))
            .await
            .unwrap();
        drop(client);
        Self { fixture, pool }
    }

    async fn connect(&self) -> Client {
        self.pool.get().await.unwrap()
    }

    async fn close(self) {
        self.pool.close();
        self.fixture.close_async().await.unwrap();
    }
}

fn text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

fn record(entries: Vec<(&str, JsonData)>) -> JsonData {
    JsonData::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn repository(client: &Client) -> UserRepository<'_, Client> {
    UserRepository {
        db: Some(client),
        writer_db: None,
        use_writer: false,
    }
}

fn ids(users: &[LiteLLMUserTable]) -> Vec<&str> {
    users.iter().map(|user| user.user_id.as_str()).collect()
}

async fn found_id<C>(users: &UserRepository<'_, C>, id: &str) -> Option<String>
where
    C: GenericClient + Sync,
{
    users.find_by_id(id).await.unwrap().map(|user| user.user_id)
}

async fn stored_email(client: &Client, user_id: &str) -> Option<String> {
    client
        .query_one(
            r#"SELECT user_email FROM "LiteLLM_UserTable" WHERE user_id = $1"#,
            &[&user_id],
        )
        .await
        .unwrap()
        .get(0)
}

fn email_page_input(extra: [&str; 2]) -> Vec<String> {
    let mut emails: Vec<String> = (0..(EMAIL_PAGE_SIZE - 1))
        .map(|index| format!("a{index:05}@x.com"))
        .collect();
    emails.extend(extra.map(str::to_owned));
    emails
}

#[test]
fn absent_user_record() {
    assert!(user_record_to_model(None).unwrap().is_none());
}

#[test]
fn three_user_json_columns() {
    let stored = record(vec![
        ("user_id", text("u")),
        ("metadata", text(r#"{"role": "admin"}"#)),
        ("model_spend", text(r#"{"gpt-4": 1.25}"#)),
        ("model_max_budget", text(r#"{"gpt-4": 10}"#)),
    ]);
    let user = user_record_to_model(Some(&stored)).unwrap().unwrap();
    assert_eq!(user.user_id, "u");
    assert_eq!(user.metadata.unwrap().0.get("role"), Some(&text("admin")));
    let model_spend = user.model_spend.0.unwrap();
    assert!(matches!(
        model_spend.0.get("gpt-4"),
        Some(JsonData::Number(value)) if value.as_f64() == Some(1.25_f64)
    ));
    let model_max_budget = user.model_max_budget.0.unwrap();
    assert!(matches!(
        model_max_budget.0.get("gpt-4"),
        Some(JsonData::Number(value)) if value.as_f64() == Some(10.0_f64)
    ));
}

#[rstest]
#[case::array_shape("[]")]
#[case::number_shape("1")]
#[case::boolean_shape("true")]
#[case::text_shape("\"text\"")]
fn nullable_json_text_and_invalid_scalar_shapes(#[case] shape: &str) {
    let nulls = record(vec![
        ("user_id", text("u")),
        ("metadata", text("null")),
        ("model_spend", text("null")),
        ("model_max_budget", text("null")),
    ]);
    let user = user_record_to_model(Some(&nulls)).unwrap().unwrap();
    assert!(user.metadata.is_none());
    assert!(user.model_spend.0.is_none());
    assert!(user.model_max_budget.0.is_none());
    let invalid = record(vec![("user_id", text("u")), ("metadata", text(shape))]);
    assert!(matches!(
        user_record_to_model(Some(&invalid)),
        Err(UserRecordError::Model { .. })
    ));
}

#[rstest]
#[case::open_brace("{")]
#[case::not_json("not json")]
#[case::missing_value("{\"a\": }")]
#[case::empty("")]
fn malformed_user_json_propagates(#[case] malformed: &str) {
    let invalid = record(vec![("user_id", text("u")), ("metadata", text(malformed))]);
    assert!(matches!(
        user_record_to_model(Some(&invalid)),
        Err(UserRecordError::Field {
            field: "metadata",
            source: ResourceRowsError::Json(_),
        })
    ));
    let truncated = record(vec![
        ("user_id", text("u")),
        ("model_spend", text(r#"{"a": 1"#)),
    ]);
    assert!(matches!(
        user_record_to_model(Some(&truncated)),
        Err(UserRecordError::Field {
            field: "model_spend",
            source: ResourceRowsError::Json(_),
        })
    ));
}

#[rstest]
#[case::teams("teams")]
#[case::models("models")]
#[case::memberships("organization_memberships")]
fn unlisted_user_fields_not_decoded(#[case] field: &str) {
    let invalid = record(vec![("user_id", text("u")), (field, text("[]"))]);
    assert!(
        matches!(
            user_record_to_model(Some(&invalid)),
            Err(UserRecordError::Model { .. })
        ),
        "field {field} must reach the model as a string"
    );
    let plain = record(vec![
        ("user_id", text("u")),
        ("user_email", text("user@example.com")),
    ]);
    assert_eq!(
        user_record_to_model(Some(&plain))
            .unwrap()
            .unwrap()
            .user_email
            .as_deref(),
        Some("user@example.com")
    );
}

#[test]
fn no_recursive_string_decode() {
    let nested = record(vec![
        ("user_id", text("u")),
        ("metadata", text(r#"{"nested": "{\"a\": 1}"}"#)),
    ]);
    let user = user_record_to_model(Some(&nested)).unwrap().unwrap();
    assert_eq!(
        user.metadata.unwrap().0.get("nested"),
        Some(&text(r#"{"a": 1}"#))
    );
}

#[test]
fn original_record_unchanged() {
    let original = record(vec![
        ("user_id", text("u")),
        ("metadata", text(r#"{"a": 1}"#)),
        ("model_spend", text(r#"{"m": 2}"#)),
    ]);
    let snapshot = original.clone();
    user_record_to_model(Some(&original)).unwrap();
    assert_eq!(original, snapshot);
}

#[test]
fn decoded_user_retains_password_exclusion() {
    let stored = record(vec![
        ("user_id", text("u")),
        ("password", text("secret-password")),
    ]);
    let user = user_record_to_model(Some(&stored)).unwrap().unwrap();
    assert_eq!(
        user.password.as_ref().unwrap().expose_secret(),
        "secret-password"
    );
    let serialized = serde_json::to_string(&user).unwrap();
    assert!(!serialized.contains("\"password\":"));
    assert!(!serialized.contains("secret-password"));
    assert!(!format!("{user:?}").contains("secret-password"));
}

#[test]
fn native_user_record_field_limits() {
    let exact = format!(r#"{{"a": "{}"}}"#, "x".repeat(FIELD_BYTES - 9));
    assert_eq!(exact.len(), FIELD_BYTES);
    let accepted = record(vec![("user_id", text("u")), ("metadata", text(&exact))]);
    assert!(user_record_to_model(Some(&accepted)).unwrap().is_some());
    let over = format!(r#"{{"a": "{}"}}"#, "x".repeat(FIELD_BYTES - 8));
    let rejected = record(vec![("user_id", text("u")), ("metadata", text(&over))]);
    assert!(matches!(
        user_record_to_model(Some(&rejected)),
        Err(UserRecordError::Field {
            field: "metadata",
            source: ResourceRowsError::TooLarge { .. },
        })
    ));
}

#[test]
fn native_user_record_field_depth_and_envelope_limits() {
    let deep = format!(
        "{}0{}",
        "[".repeat(FIELD_DEPTH + 1),
        "]".repeat(FIELD_DEPTH + 1)
    );
    let too_deep = record(vec![("user_id", text("u")), ("metadata", text(&deep))]);
    assert!(matches!(
        user_record_to_model(Some(&too_deep)),
        Err(UserRecordError::Field {
            field: "metadata",
            source: ResourceRowsError::TooDeep,
        })
    ));
    let at_limit = format!("{}0{}", "[".repeat(FIELD_DEPTH), "]".repeat(FIELD_DEPTH));
    let accepted = record(vec![("user_id", text("u")), ("metadata", text(&at_limit))]);
    assert!(matches!(
        user_record_to_model(Some(&accepted)),
        Err(UserRecordError::Model { .. })
    ));
    let oversized = vec![b' '; 0x0010_0001];
    assert!(matches!(
        decode_user_record(&oversized),
        Err(UserRecordError::Record {
            source: ResourceRowsError::TooLarge { .. },
        })
    ));
    let deep_record = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert!(matches!(
        decode_user_record(deep_record.as_bytes()),
        Err(UserRecordError::Record {
            source: ResourceRowsError::TooDeep,
        })
    ));
    let valid = br#"{"user_id": "u", "metadata": "{\"a\": 1}"}"#;
    assert_eq!(decode_user_record(valid).unwrap().unwrap().user_id, "u");
}

#[rstest]
fn replays_the_user_record_corpus(#[files("fuzz/corpus/user_record/*")] path: PathBuf) {
    drop(decode_user_record(&fs::read(path).unwrap()));
}

#[test]
fn email_exact_set_then_sort() {
    let pages = email_pages(&[
        "b@x".to_owned(),
        "A@x".to_owned(),
        "b@x".to_owned(),
        "B@x".to_owned(),
    ]);
    assert_eq!(
        pages,
        vec![vec!["A@x".to_owned(), "B@x".to_owned(), "b@x".to_owned()]]
    );
    let ordered = email_pages(&["z@x".to_owned(), "a@x".to_owned(), "m@x".to_owned()]);
    assert_eq!(
        ordered,
        vec![vec!["a@x".to_owned(), "m@x".to_owned(), "z@x".to_owned()]]
    );
}

#[test]
fn email_five_thousand_page_boundary() {
    let addresses = |count: usize| -> Vec<String> {
        (0..count)
            .map(|index| format!("user{index:05}@example.com"))
            .collect()
    };
    let exact = email_pages(&addresses(EMAIL_PAGE_SIZE));
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].len(), EMAIL_PAGE_SIZE);
    let over = email_pages(&addresses(EMAIL_PAGE_SIZE + 1));
    assert_eq!(over.len(), 2);
    assert_eq!(over[0].len(), EMAIL_PAGE_SIZE);
    assert_eq!(over[1].len(), 1);
    let doubled = email_pages(&addresses(2 * EMAIL_PAGE_SIZE));
    assert_eq!(doubled.len(), 2);
    assert_eq!(doubled[1].len(), EMAIL_PAGE_SIZE);
}

#[tokio::test]
async fn native_user_membership_lookup() {
    let database = Database::start().await;
    let client = database.connect().await;
    client
        .batch_execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('alpha'), ('bravo'), ('charlie')"#,
        )
        .await
        .unwrap();
    let users = Repository {
        client: Some(&client),
        table: USER_TABLE,
    }
    .find_many_in(
        KeyColumn::Id,
        vec!["alpha".to_owned(), "bravo".to_owned(), "missing".to_owned()],
    )
    .await
    .unwrap();
    let mut found = ids(&users);
    found.sort_unstable();
    assert_eq!(found, vec!["alpha", "bravo"]);
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn email_empty_skips_missing_client() {
    let absent: UserRepository<'_, Client> = UserRepository {
        db: None,
        writer_db: None,
        use_writer: false,
    };
    assert!(absent.find_by_emails(&[]).await.unwrap().is_empty());
    let error = absent
        .find_by_emails(&["user@example.com".to_owned()])
        .await
        .unwrap_err();
    assert!(matches!(error, DbError::NoDbConnected));
}

#[tokio::test]
async fn user_writer_selection_and_default_ids() {
    let reader_database = Database::start().await;
    let writer_database = Database::start().await;
    let reader = reader_database.connect().await;
    let writer = writer_database.connect().await;
    reader.batch_execute(READER_SEED).await.unwrap();
    writer.batch_execute(WRITER_SEED).await.unwrap();
    let reading = UserRepository {
        db: Some(&reader),
        writer_db: Some(&writer),
        use_writer: false,
    };
    assert_eq!(
        found_id(&reading, "id-value").await,
        Some("id-value".to_owned())
    );
    assert_eq!(found_id(&reading, "alias-value").await, None);
    let writing = UserRepository {
        db: Some(&reader),
        writer_db: Some(&writer),
        use_writer: true,
    };
    assert_eq!(
        found_id(&writing, "writer-only").await,
        Some("writer-only".to_owned())
    );
    assert_eq!(found_id(&writing, "id-value").await, None);
    let fallback: UserRepository<'_, Client> = UserRepository {
        db: Some(&reader),
        writer_db: None,
        use_writer: true,
    };
    assert_eq!(
        found_id(&fallback, "id-value").await,
        Some("id-value".to_owned())
    );
    drop(reader);
    drop(writer);
    reader_database.close().await;
    writer_database.close().await;
}

#[tokio::test]
async fn organization_project_default_ids() {
    let database = Database::start().await;
    let client = database.connect().await;
    client
        .batch_execute(ORGANIZATION_PROJECT_SEED)
        .await
        .unwrap();
    let organizations = OrganizationRepository {
        db: Some(&client),
        table: ORGANIZATION_TABLE,
    };
    let found = organizations.find_by_id("org-1").await.unwrap().unwrap();
    assert_eq!(found.organization_id.as_deref(), Some("org-1"));
    assert_eq!(found.budget_id, "budget-1");
    assert!(organizations.find_by_id("alias-1").await.unwrap().is_none());
    assert!(organizations.find_by_id("missing").await.unwrap().is_none());
    let projects = ProjectRepository {
        db: Some(&client),
        table: PROJECT_TABLE,
    };
    let project = projects.find_by_id("project-1").await.unwrap().unwrap();
    assert_eq!(project.project_id, "project-1");
    assert_eq!(project.project_alias.as_deref(), Some("project-alias-1"));
    assert!(
        projects
            .find_by_id("project-alias-1")
            .await
            .unwrap()
            .is_none()
    );
    assert!(projects.find_by_id("missing").await.unwrap().is_none());
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn missing_rows_and_errors_preserved() {
    let database = Database::start().await;
    let client = database.connect().await;
    client
        .batch_execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id, metadata)
               VALUES ('broken', '"not-json"'::jsonb)"#,
        )
        .await
        .unwrap();
    let users = repository(&client);
    assert!(users.find_by_id("missing").await.unwrap().is_none());
    let decode_error = users.find_by_id("broken").await.unwrap_err();
    assert!(matches!(
        decode_error,
        DbError::InvalidRecord {
            source: RowDecodeError::User {
                source: UserRecordError::Field {
                    field: "metadata",
                    source: ResourceRowsError::Json(_),
                }
            }
        }
    ));
    client
        .batch_execute(r#"DROP TABLE "LiteLLM_UserTable" CASCADE"#)
        .await
        .unwrap();
    let query_error = users.find_by_id("missing").await.unwrap_err();
    assert!(matches!(query_error, DbError::SqlState { .. }));
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn postgres_email_matching_literal_case_insensitive() {
    let database = Database::start().await;
    let client = database.connect().await;
    client.batch_execute(EMAIL_MATCH_SEED).await.unwrap();
    let users = repository(&client);
    let alice = users
        .find_by_emails(&["ALICE@example.COM".to_owned()])
        .await
        .unwrap();
    assert_eq!(ids(&alice), vec!["alice"]);
    let underscore = users
        .find_by_emails(&["uNdEr_ScOrE@ExAmPlE.CoM".to_owned()])
        .await
        .unwrap();
    assert_eq!(ids(&underscore), vec!["underscore"]);
    let percent = users
        .find_by_emails(&["percent%x@example.com".to_owned()])
        .await
        .unwrap();
    assert_eq!(ids(&percent), vec!["percent"]);
    let absent = users
        .find_by_emails(&["alice@example.org".to_owned()])
        .await
        .unwrap();
    assert!(absent.is_empty());
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn email_page_order_and_duplicate_output() {
    let database = Database::start().await;
    let client = database.connect().await;
    client
        .batch_execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES ('dup', 'dup@x.com')"#,
        )
        .await
        .unwrap();
    let users = repository(&client);
    let found = users
        .find_by_emails(&email_page_input(["Dup@x.com", "dup@x.com"]))
        .await
        .unwrap();
    assert_eq!(ids(&found), vec!["dup", "dup"]);
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn email_later_page_error_no_partial_result() {
    let database = Database::start().await;
    let client = database.connect().await;
    client.batch_execute(EMAIL_LATER_PAGE_SEED).await.unwrap();
    let users = repository(&client);
    let error = users
        .find_by_emails(&email_page_input(["first@x.com", "z-last@x.com"]))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        DbError::InvalidRecord {
            source: RowDecodeError::User {
                source: UserRecordError::Field {
                    field: "metadata",
                    source: ResourceRowsError::Json(_),
                }
            }
        }
    ));
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn postgres_shadow_placeholder_predicates() {
    let database = Database::start().await;
    let client = database.connect().await;
    client
        .batch_execute(PLACEHOLDER_PREDICATES_SEED)
        .await
        .unwrap();
    let users = repository(&client);
    let placeholders = users.find_shadowing_placeholders(&client).await.unwrap();
    assert_eq!(placeholders.len(), 2);
    assert_eq!(placeholders[0].placeholder_user_id, "placeholder-1");
    assert_eq!(placeholders[0].resolved_user_ids, vec!["resolver-1"]);
    assert_eq!(placeholders[0].team_ids, vec!["t1"]);
    assert_eq!(placeholders[1].placeholder_user_id, "placeholder-2");
    assert_eq!(placeholders[1].resolved_user_ids, vec!["resolver-2"]);
    assert!(placeholders[1].team_ids.is_empty());
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn placeholder_or_match_not_duplicated() {
    let database = Database::start().await;
    let client = database.connect().await;
    client.batch_execute(PLACEHOLDER_OR_SEED).await.unwrap();
    let users = repository(&client);
    let placeholders = users.find_shadowing_placeholders(&client).await.unwrap();
    assert_eq!(placeholders.len(), 1);
    assert_eq!(
        placeholders[0].resolved_user_ids,
        vec!["resolver-both", "resolver-email"]
    );
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn placeholder_order_and_team_order() {
    let database = Database::start().await;
    let client = database.connect().await;
    client.batch_execute(PLACEHOLDER_ORDER_SEED).await.unwrap();
    let users = repository(&client);
    let placeholders = users.find_shadowing_placeholders(&client).await.unwrap();
    assert_eq!(placeholders.len(), 2);
    assert_eq!(placeholders[0].placeholder_user_id, "placeholder-a");
    assert_eq!(placeholders[0].team_ids, vec!["t0"]);
    assert_eq!(placeholders[0].resolved_user_ids, vec!["resolver-a"]);
    assert_eq!(placeholders[1].placeholder_user_id, "placeholder-z");
    assert_eq!(placeholders[1].team_ids, vec!["t2", "t1", "t2"]);
    assert_eq!(
        placeholders[1].resolved_user_ids,
        vec!["resolver-z-alpha", "resolver-z-bravo"]
    );
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn placeholder_uses_explicit_transaction() {
    let database = Database::start().await;
    let mut writer = database.connect().await;
    let other = database.connect().await;
    let transaction = writer.transaction().await.unwrap();
    transaction
        .batch_execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('placeholder-tx');
               INSERT INTO "LiteLLM_UserTable" (user_id, sso_user_id)
               VALUES ('resolver-tx', 'placeholder-tx')"#,
        )
        .await
        .unwrap();
    let users = repository(&other);
    let placeholders = users
        .find_shadowing_placeholders(&transaction)
        .await
        .unwrap();
    assert_eq!(placeholders.len(), 1);
    assert_eq!(placeholders[0].placeholder_user_id, "placeholder-tx");
    assert_eq!(placeholders[0].resolved_user_ids, vec!["resolver-tx"]);
    let visible: i64 = other
        .query_one(r#"SELECT count(*) FROM "LiteLLM_UserTable""#, &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(visible, 0);
    transaction.rollback().await.unwrap();

    drop(writer);
    drop(other);
    database.close().await;
}

#[tokio::test]
async fn placeholder_null_teams_validation_failure() {
    let database = Database::start().await;
    let client = database.connect().await;
    client
        .batch_execute(PLACEHOLDER_NULL_TEAMS_SEED)
        .await
        .unwrap();
    let users = repository(&client);
    let error = users
        .find_shadowing_placeholders(&client)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        DbError::InvalidRecord {
            source: PlaceholderError::MissingField { field: "team_ids" }
        }
    ));
    drop(client);
    database.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_email_backfill_race() {
    let database = Database::start().await;
    let client = database.connect().await;
    client.batch_execute(RACE_SEED).await.unwrap();
    let mut first = database.connect().await;
    let second = database.connect().await;
    let transaction = first.transaction().await.unwrap();
    let first_users = UserRepository {
        db: Some(&transaction),
        writer_db: None,
        use_writer: false,
    };
    assert_eq!(
        first_users
            .backfill_null_user_email("race", "first@example.com")
            .await
            .unwrap(),
        1
    );
    let second_users = repository(&second);
    let mut pending = Box::pin(second_users.backfill_null_user_email("race", "second@example.com"));
    let waited = timeout(Duration::from_millis(300), &mut pending)
        .await
        .is_err();
    assert!(
        waited,
        "the second backfill must wait for the first transaction"
    );
    transaction.commit().await.unwrap();
    assert_eq!(pending.await.unwrap(), 0);
    assert_eq!(
        stored_email(&client, "race").await.as_deref(),
        Some("first@example.com")
    );
    drop(first);
    drop(second);
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn existing_empty_or_different_email_unchanged() {
    let database = Database::start().await;
    let client = database.connect().await;
    client
        .batch_execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES
               ('empty', ''), ('different', 'old@example.com')"#,
        )
        .await
        .unwrap();
    let users = repository(&client);
    assert_eq!(
        users
            .backfill_null_user_email("empty", "new@example.com")
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        users
            .backfill_null_user_email("different", "new@example.com")
            .await
            .unwrap(),
        0
    );
    assert_eq!(stored_email(&client, "empty").await.as_deref(), Some(""));
    assert_eq!(
        stored_email(&client, "different").await.as_deref(),
        Some("old@example.com")
    );
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn backfill_missing_row_zero() {
    let database = Database::start().await;
    let client = database.connect().await;
    let users = repository(&client);
    assert_eq!(
        users
            .backfill_null_user_email("absent", "new@example.com")
            .await
            .unwrap(),
        0
    );
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn backfill_supplied_email_verbatim() {
    let database = Database::start().await;
    let client = database.connect().await;
    client
        .batch_execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES ('verbatim', NULL)"#,
        )
        .await
        .unwrap();
    let users = repository(&client);
    let supplied = "  Mixed+CASE@Example.COM  ";
    assert_eq!(
        users
            .backfill_null_user_email("verbatim", supplied)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        stored_email(&client, "verbatim").await.as_deref(),
        Some(supplied)
    );
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn postgres_billable_json_boolean_only() {
    let database = Database::start().await;
    let client = database.connect().await;
    client.batch_execute(BILLABLE_SEED).await.unwrap();
    let users = repository(&client);
    assert_eq!(users.count_billable_users().await.unwrap(), 7);
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn billable_two_reads_and_concurrent_zero_floor() {
    let database = Database::start().await;
    let client = database.connect().await;
    client.batch_execute(TWO_READS_SEED).await.unwrap();
    let users = repository(&client);
    assert_eq!(users.count_billable_users().await.unwrap(), 0);
    drop(client);
    database.close().await;
}

#[tokio::test]
async fn count_error_propagates() {
    let database = Database::start().await;
    let client = database.connect().await;
    client
        .batch_execute(r#"DROP TABLE "LiteLLM_UserTable" CASCADE"#)
        .await
        .unwrap();
    let users = repository(&client);
    let count_error = users.count_billable_users().await.unwrap_err();
    assert!(matches!(count_error, DbError::SqlState { .. }));
    let backfill_error = users
        .backfill_null_user_email("user", "user@example.com")
        .await
        .unwrap_err();
    assert!(matches!(backfill_error, DbError::SqlState { .. }));
    drop(client);
    database.close().await;
}

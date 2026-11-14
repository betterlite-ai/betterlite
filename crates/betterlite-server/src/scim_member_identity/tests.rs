#![cfg(test)]

use std::collections::BTreeSet;
use std::io;
use std::str;
use std::sync::{Arc, Mutex};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use http::StatusCode;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tracing_subscriber::fmt::MakeWriter;

use super::classified::{
    ClassifiedGroupMember, SkipReason, SkippedGroupMember, admitted_member_ids,
    partition_classified_members, roster_entries_named_by,
};
use super::error::ScimMemberIdentityError;
use super::lookups::{
    accounts_named_by_member_value, is_scim_managed_team, users_named_by_member_value,
};
use super::member_value::{
    json_object_fields, member_value, metadata_has_scim_provenance, normalized_member_type,
};
use super::resolver::MemberResolver;
use crate::scim_records::group::{Member, MemberKind};

fn json_data(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn member(value: &str, kind: Option<&str>) -> Member {
    Member {
        value: value.to_owned(),
        display: None,
        kind: MemberKind(kind.map(str::to_owned)),
    }
}

fn provenance(raw: &JsonData) -> bool {
    json_object_fields(raw).is_some_and(metadata_has_scim_provenance)
}

fn resolved(user_id: &str) -> ClassifiedGroupMember {
    ClassifiedGroupMember::Resolved {
        user_id: user_id.to_owned(),
    }
}

fn unknown(value: &str) -> ClassifiedGroupMember {
    ClassifiedGroupMember::Unknown {
        value: value.to_owned(),
    }
}

fn ambiguous(value: &str) -> ClassifiedGroupMember {
    ClassifiedGroupMember::Ambiguous {
        value: value.to_owned(),
    }
}

fn skipped(value: &str, reason: SkipReason) -> ClassifiedGroupMember {
    ClassifiedGroupMember::Skipped {
        value: value.to_owned(),
        reason,
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

fn set(values: &[&str]) -> BTreeSet<String> {
    strings(values).into_iter().collect()
}

#[derive(Clone)]
struct CaptureWriter(Arc<Mutex<String>>);

impl io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = str::from_utf8(buf)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        self.0
            .lock()
            .map_err(|source| io::Error::other(source.to_string()))?
            .push_str(text);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CaptureWriter {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

#[rstest]
#[case::empty("", None)]
#[case::blank("   ", None)]
#[case::separators("\u{1c}\u{1f}", None)]
#[case::spaced(" u1 ", Some(" u1 "))]
#[case::plain("u1", Some("u1"))]
fn scim_member_identity_member_value_keeps_the_original_spelling(
    #[case] value: &str,
    #[case] expected: Option<&str>,
) {
    let candidate = member(value, None);
    let checked = member_value(&candidate);
    assert_eq!(checked.ok(), expected);
}

#[rstest]
#[case::absent(None, None)]
#[case::empty(Some(""), None)]
#[case::blank(Some("  "), None)]
#[case::separator(Some("\u{1f}"), None)]
#[case::canonical(Some("User"), Some("user"))]
#[case::trimmed(Some(" GROUP "), Some("group"))]
#[case::noncanonical(Some("direct"), Some("direct"))]
fn scim_member_identity_normalized_member_type_folds_and_trims(
    #[case] declared: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let normalized = normalized_member_type(&member("u1", declared));
    assert_eq!(normalized.as_deref(), expected);
}

#[rstest]
#[case::object("{}", true)]
#[case::filled("{\"scim_managed\":true}", true)]
#[case::null("null", false)]
#[case::flag("false", false)]
#[case::number("0", false)]
#[case::text("\"text\"", false)]
#[case::array("[]", false)]
fn scim_member_identity_json_object_fields_accepts_only_objects(
    #[case] raw: &str,
    #[case] expected: bool,
) {
    assert_eq!(json_object_fields(&json_data(raw)).is_some(), expected);
}

#[rstest]
#[case::empty("{}", false)]
#[case::managed("{\"scim_managed\":true}", true)]
#[case::unmanaged("{\"scim_managed\":false}", false)]
#[case::zero("{\"scim_managed\":0}", false)]
#[case::blank("{\"scim_managed\":\"\"}", false)]
#[case::text("{\"scim_managed\":\"yes\"}", true)]
#[case::data_null("{\"scim_data\":null}", false)]
#[case::data_false("{\"scim_data\":false}", true)]
#[case::data_array("{\"scim_data\":[]}", true)]
#[case::data_object("{\"scim_data\":{}}", true)]
#[case::data_text("{\"scim_data\":\"x\"}", true)]
#[case::array("[]", false)]
#[case::null("null", false)]
fn scim_member_identity_provenance_reads_both_scim_markers(
    #[case] raw: &str,
    #[case] expected: bool,
) {
    assert_eq!(provenance(&json_data(raw)), expected);
}

#[test]
fn scim_member_identity_partition_keeps_order_and_duplicates() {
    let classified = [
        resolved("u1"),
        resolved("u1"),
        unknown("u9"),
        skipped("g1", SkipReason::NestedGroup),
        ambiguous("a1"),
        skipped("g1", SkipReason::NonUserType),
        unknown("u9"),
    ];
    let partition = partition_classified_members(&classified);
    assert_eq!(partition.resolved_ids, strings(&["u1", "u1"]));
    assert_eq!(
        partition.skipped,
        [
            SkippedGroupMember {
                value: "g1".to_owned(),
                reason: SkipReason::NestedGroup,
            },
            SkippedGroupMember {
                value: "g1".to_owned(),
                reason: SkipReason::NonUserType,
            },
        ]
    );
    assert_eq!(partition.unknown_ids, strings(&["u9", "u9"]));
    assert_eq!(partition.ambiguous_values, strings(&["a1"]));
}

#[test]
fn scim_member_identity_admitted_ids_keep_resolved_and_created_once() {
    let created = set(&["ghost"]);
    let classified = [
        resolved("u1"),
        resolved("u1"),
        unknown("ghost"),
        unknown("absent"),
        skipped("g1", SkipReason::NestedGroup),
        ambiguous("a1"),
    ];
    assert_eq!(
        admitted_member_ids(&classified, &created),
        strings(&["u1", "ghost"])
    );
}

#[test]
fn scim_member_identity_roster_entries_keep_the_literal_and_resolved_ids() {
    let roster = set(&["shared", "other", "both"]);
    assert_eq!(
        roster_entries_named_by("shared", &roster, &strings(&["other", "absent"])),
        strings(&["shared", "other"])
    );
    assert_eq!(
        roster_entries_named_by("absent", &roster, &strings(&["other"])),
        strings(&["other"])
    );
    assert_eq!(
        roster_entries_named_by("both", &roster, &strings(&["both"])),
        strings(&["both"])
    );
    assert!(roster_entries_named_by("absent", &roster, &[]).is_empty());
}

#[rstest]
#[case::empty(
    ScimMemberIdentityError::EmptyMemberValue,
    StatusCode::BAD_REQUEST,
    "Invalid member: user ID cannot be empty."
)]
#[case::ambiguous(
    ScimMemberIdentityError::AmbiguousRemoval { value: "a@b".to_owned() },
    StatusCode::BAD_REQUEST,
    "Member ID 'a@b' names more than one member of this group, so the removal cannot be attributed. Send the LiteLLM user ID as the member value, or resolve the duplicate."
)]
#[case::no_database(
    ScimMemberIdentityError::NoDatabase,
    StatusCode::INTERNAL_SERVER_ERROR,
    "No database connected"
)]
fn scim_member_identity_error_status_and_detail(
    #[case] error: ScimMemberIdentityError,
    #[case] status: StatusCode,
    #[case] detail: &str,
) {
    assert_eq!(error.status_code(), status);
    assert_eq!(error.to_string(), detail);
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_UserTable" (
    user_id TEXT PRIMARY KEY,
    sso_user_id TEXT,
    user_email TEXT
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    metadata JSONB
);
INSERT INTO "LiteLLM_UserTable" (user_id, sso_user_id, user_email) VALUES
    ('u1', 'sub-1', 'ann@example.com'),
    ('u2', 'sub-2', 'ann@example.com'),
    ('u3', NULL, 'bob@example.com'),
    ('shared', NULL, NULL),
    ('u_other', 'shared', NULL),
    ('aa', 'zzz', NULL),
    ('bb', NULL, 'zzz'),
    ('zzz', NULL, 'unrelated@example.com');
INSERT INTO "LiteLLM_TeamTable" (team_id, metadata) VALUES
    ('team-managed', '{"scim_managed": true}'),
    ('team-data', '{"scim_data": {"displayName": "x"}}'),
    ('team-data-null', '{"scim_data": null}'),
    ('team-false', '{"scim_managed": false}'),
    ('team-plain', '{"other": 1}'),
    ('team-encoded', '"{\"scim_managed\": true}"');
"#;

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["scim-member-identity-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let client = pool.get().await.unwrap();
    client.batch_execute(SCHEMA).await.unwrap();
    drop(client);
    (fixture, pool)
}

fn store(pool: &Pool) -> PostgresStore {
    PostgresStore {
        pool: Some(pool.clone()),
    }
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.unwrap();
}

fn assert_no_database(error: &ScimMemberIdentityError) {
    assert!(matches!(error, ScimMemberIdentityError::NoDatabase));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(error.to_string(), "No database connected");
}

#[tokio::test]
async fn scim_member_identity_users_named_by_member_value_trims_and_folds_case() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    assert_eq!(
        users_named_by_member_value(&store, " sub-1 ", Some(2))
            .await
            .unwrap(),
        strings(&["u1"])
    );
    assert_eq!(
        users_named_by_member_value(&store, " ann@example.com ", Some(2))
            .await
            .unwrap(),
        strings(&["u1", "u2"])
    );
    assert!(
        users_named_by_member_value(&store, "nobody", Some(2))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        users_named_by_member_value(&store, "bob@example.com", None)
            .await
            .unwrap(),
        strings(&["u3"])
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_users_named_by_member_value_bounds_the_scan() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let bounded = users_named_by_member_value(&store, "ann@example.com", Some(1))
        .await
        .unwrap();
    assert_eq!(bounded.len(), 1);
    assert_eq!(
        users_named_by_member_value(&store, "ann@example.com", None)
            .await
            .unwrap(),
        strings(&["u1", "u2"])
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_accounts_named_by_member_value_reads_a_keyed_row() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    assert_eq!(
        accounts_named_by_member_value(&store, "zzz").await.unwrap(),
        strings(&["zzz", "aa", "bb"])
    );
    assert_eq!(
        accounts_named_by_member_value(&store, "u3").await.unwrap(),
        strings(&["u3"])
    );
    assert_eq!(
        accounts_named_by_member_value(&store, "shared")
            .await
            .unwrap(),
        strings(&["shared", "u_other"])
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_classify_nested_group_never_looks_the_value_up() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    assert_eq!(
        resolver
            .classify_group_member(&member("u3", Some("Group")))
            .await
            .unwrap(),
        skipped("u3", SkipReason::NestedGroup)
    );
    assert_eq!(
        resolver
            .classify_group_member(&member(" team-managed ", Some(" GROUP ")))
            .await
            .unwrap(),
        skipped(" team-managed ", SkipReason::NestedGroup)
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_classify_resolves_a_unique_identity_or_email() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    assert_eq!(
        resolver
            .classify_group_member(&member(" sub-1 ", None))
            .await
            .unwrap(),
        resolved("u1")
    );
    assert_eq!(
        resolver
            .classify_group_member(&member(" bob@example.com", Some("User")))
            .await
            .unwrap(),
        resolved("u3")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_classify_a_unique_user_id_outranks_a_noncanonical_type() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    assert_eq!(
        resolver
            .classify_group_member(&member("u3", Some("direct")))
            .await
            .unwrap(),
        resolved("u3")
    );
    assert_eq!(
        resolver
            .classify_group_member(&member("shared", Some("direct")))
            .await
            .unwrap(),
        ambiguous("shared")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_classify_skips_non_user_types_before_resolving_identities() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    assert_eq!(
        resolver
            .classify_group_member(&member("bob@example.com", Some("direct")))
            .await
            .unwrap(),
        skipped("bob@example.com", SkipReason::NonUserType)
    );
    assert_eq!(
        resolver
            .classify_group_member(&member(" sub-1 ", Some(" Direct ")))
            .await
            .unwrap(),
        skipped(" sub-1 ", SkipReason::NonUserType)
    );
    assert_eq!(
        resolver
            .classify_group_member(&member("ghost", Some("direct")))
            .await
            .unwrap(),
        skipped("ghost", SkipReason::NonUserType)
    );
    assert_eq!(
        resolver
            .classify_group_member(&member("ghost", Some("User")))
            .await
            .unwrap(),
        unknown("ghost")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_classify_a_shared_identity_is_ambiguous() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    assert_eq!(
        resolver
            .classify_group_member(&member("ann@example.com", None))
            .await
            .unwrap(),
        ambiguous("ann@example.com")
    );
    assert_eq!(
        resolver
            .classify_group_member(&member("zzz", None))
            .await
            .unwrap(),
        ambiguous("zzz")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_classify_untyped_managed_teams_skip() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    assert_eq!(
        resolver
            .classify_group_member(&member("team-managed", None))
            .await
            .unwrap(),
        skipped("team-managed", SkipReason::ExistingTeam)
    );
    assert_eq!(
        resolver
            .classify_group_member(&member("team-data", None))
            .await
            .unwrap(),
        skipped("team-data", SkipReason::ExistingTeam)
    );
    assert_eq!(
        resolver
            .classify_group_member(&member("team-encoded", None))
            .await
            .unwrap(),
        skipped("team-encoded", SkipReason::ExistingTeam)
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_classify_untyped_unmanaged_teams_stay_unknown() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    assert_eq!(
        resolver
            .classify_group_member(&member("team-data-null", None))
            .await
            .unwrap(),
        unknown("team-data-null")
    );
    assert_eq!(
        resolver
            .classify_group_member(&member("team-false", None))
            .await
            .unwrap(),
        unknown("team-false")
    );
    assert_eq!(
        resolver
            .classify_group_member(&member("team-plain", None))
            .await
            .unwrap(),
        unknown("team-plain")
    );
    assert_eq!(
        resolver
            .classify_group_member(&member("team-managed", Some("User")))
            .await
            .unwrap(),
        unknown("team-managed")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_classification_events_name_the_value_and_account() {
    let (fixture, pool) = database().await;
    let records = Arc::new(Mutex::new(String::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(CaptureWriter(Arc::clone(&records)))
        .finish();
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    let guard = tracing::subscriber::set_default(subscriber);
    let matched = resolver.classify_group_member(&member("sub-2", None)).await;
    let collided = resolver
        .classify_group_member(&member("shared", None))
        .await;
    let shared_by_two = resolver
        .classify_group_member(&member("ann@example.com", None))
        .await;
    drop(guard);
    close(fixture, pool).await;
    assert_eq!(matched.unwrap(), resolved("u2"));
    assert_eq!(collided.unwrap(), ambiguous("shared"));
    assert_eq!(shared_by_two.unwrap(), ambiguous("ann@example.com"));
    let captured = records.lock().unwrap().clone();
    assert!(captured.contains("SCIM: member matched a user by SSO or email"));
    assert!(captured.contains("value=sub-2"));
    assert!(captured.contains("user_id=u2"));
    assert!(captured.contains("SCIM: member is also another account's identity or email"));
    assert!(captured.contains("value=shared"));
    assert!(captured.contains("other=u_other"));
    assert!(captured.contains("SCIM: member names several accounts and is ambiguous"));
    assert!(captured.contains("value=ann@example.com"));
}

#[tokio::test]
async fn scim_member_identity_empty_member_value_is_a_400_before_any_lookup() {
    let missing = PostgresStore { pool: None };
    let resolver = MemberResolver { store: &missing };
    let error = resolver
        .classify_group_member(&member("   ", None))
        .await
        .unwrap_err();
    assert!(matches!(error, ScimMemberIdentityError::EmptyMemberValue));
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(
        error.to_string(),
        "Invalid member: user ID cannot be empty."
    );
}

#[tokio::test]
async fn scim_member_identity_every_lookup_without_a_database_is_a_500() {
    let missing = PostgresStore { pool: None };
    let resolver = MemberResolver { store: &missing };
    assert_no_database(
        &users_named_by_member_value(&missing, "u1", Some(2))
            .await
            .err()
            .unwrap(),
    );
    assert_no_database(
        &accounts_named_by_member_value(&missing, "u1")
            .await
            .err()
            .unwrap(),
    );
    assert_no_database(&is_scim_managed_team(&missing, "team").await.err().unwrap());
    assert_no_database(
        &resolver
            .classify_group_member(&member("u1", None))
            .await
            .err()
            .unwrap(),
    );
    assert_no_database(
        &resolver
            .member_ids_to_drop(&[member("u1", None)], &BTreeSet::new())
            .await
            .err()
            .unwrap(),
    );
}

#[tokio::test]
async fn scim_member_identity_member_ids_to_drop_resolves_a_shared_email_to_one_member() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    let dropped = resolver
        .member_ids_to_drop(&[member("ann@example.com", None)], &set(&["u1"]))
        .await
        .unwrap();
    assert_eq!(dropped, set(&["u1"]));
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_member_ids_to_drop_keeps_a_literal_roster_id() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    let dropped = resolver
        .member_ids_to_drop(&[member("placeholder-id", None)], &set(&["placeholder-id"]))
        .await
        .unwrap();
    assert_eq!(dropped, set(&["placeholder-id"]));
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_member_ids_to_drop_refuses_two_roster_members() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    let error = resolver
        .member_ids_to_drop(&[member("shared", None)], &set(&["shared", "u_other"]))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ScimMemberIdentityError::AmbiguousRemoval { .. }
    ));
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(
        error.to_string(),
        "Member ID 'shared' names more than one member of this group, so the removal cannot be attributed. Send the LiteLLM user ID as the member value, or resolve the duplicate."
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_member_ids_to_drop_ignores_an_external_collision() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    let dropped = resolver
        .member_ids_to_drop(&[member("shared", None)], &set(&["u_other"]))
        .await
        .unwrap();
    assert_eq!(dropped, set(&["u_other"]));
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_member_ids_to_drop_deduplicates_written_values() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    let members = [
        member("bob@example.com", None),
        member("bob@example.com", None),
    ];
    let dropped = resolver
        .member_ids_to_drop(&members, &set(&["u3"]))
        .await
        .unwrap();
    assert_eq!(dropped, set(&["u3"]));
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_member_ids_to_drop_skips_a_value_naming_nobody() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let resolver = MemberResolver { store: &store };
    let members = [member("ann@example.com", None), member("ghost", None)];
    let dropped = resolver
        .member_ids_to_drop(&members, &set(&["u1"]))
        .await
        .unwrap();
    assert_eq!(dropped, set(&["u1"]));
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_member_identity_member_ids_to_drop_empty_value_is_a_400() {
    let missing = PostgresStore { pool: None };
    let resolver = MemberResolver { store: &missing };
    let error = resolver
        .member_ids_to_drop(&[member("", None)], &BTreeSet::new())
        .await
        .unwrap_err();
    assert!(matches!(error, ScimMemberIdentityError::EmptyMemberValue));
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
}

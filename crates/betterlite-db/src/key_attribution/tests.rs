#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_postgres_fixture::connection::PostgresFixture;
use rstest::rstest;
use sha2::{Digest as _, Sha256};

use crate::budget_permission_rows::JsonData;
use crate::postgres_store::session::PostgresStore;

use super::error::KeyAttributionError;
use super::metadata::{
    KeyMetadata, UserDetails, attach_details, cli_candidates, cli_metadata, cli_user_id,
    meta_with_user_details, needed_user_ids, only_team, unanimous, user_details,
    user_id_needing_details,
};
use super::projection::{ExportFields, missing_keys, project_rows, row_with_recovered_fields};
use super::recovery::{
    attach_user_details, fill_missing_api_key_aliases, recover_cli_session_keys,
    recover_daily_spend_owners, recover_double_hashed_keys,
};
use super::rows::{
    DailyOwnerRow, TokenDigestRow, daily_owners, digest_keys, token_metadata, unresolved_digests,
};

#[rstest]
#[case::both_absent(None, None, None)]
#[case::first_only(Some("owner"), None, Some("owner"))]
#[case::last_only(None, Some("owner"), Some("owner"))]
#[case::agreeing(Some("owner"), Some("owner"), Some("owner"))]
#[case::conflicting(Some("first"), Some("last"), None)]
#[case::empty_agrees(Some(""), Some(""), Some(""))]
#[case::empty_conflicts(Some(""), Some("owner"), None)]
fn unanimity_takes_one_side_or_matching_values(
    #[case] first: Option<&str>,
    #[case] last: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(unanimous(first, last), expected.map(str::to_owned));
}

#[rstest]
#[case::exact_prefix("cli-session-", false)]
#[case::with_user("cli-session-user", true)]
#[case::double_prefix("cli-session-cli-session-u", true)]
#[case::wrong_prefix("sk-cli-session-u", false)]
#[case::missing_dash("cli-sessionu", false)]
#[case::uppercase("CLI-SESSION-u", false)]
#[case::empty("", false)]
fn cli_session_keys_need_a_nonempty_suffix(#[case] key: &str, #[case] expected: bool) {
    assert_eq!(cli_user_id(key).is_some(), expected);
}

#[test]
fn cli_candidates_strip_one_prefix_and_keep_the_full_key() {
    let keys = BTreeSet::from([
        "cli-session-user".to_owned(),
        "cli-session-".to_owned(),
        "sk-user".to_owned(),
    ]);
    let candidates = cli_candidates(&keys);
    assert_eq!(
        candidates,
        BTreeMap::from([("cli-session-user".to_owned(), "user".to_owned())])
    );
}

#[test]
fn cli_metadata_keeps_only_known_users_with_the_full_key_as_alias() {
    let candidates = BTreeMap::from([
        ("cli-session-known".to_owned(), "known".to_owned()),
        ("cli-session-missing".to_owned(), "missing".to_owned()),
    ]);
    let details = BTreeMap::from([("known".to_owned(), user("known@example.com", &[]))]);
    let recovered = cli_metadata(&candidates, &details);
    assert_eq!(
        recovered,
        BTreeMap::from([(
            "cli-session-known".to_owned(),
            KeyMetadata {
                key_alias: Some("cli-session-known".to_owned()),
                user_id: Some("known".to_owned()),
                ..KeyMetadata::default()
            }
        )])
    );
}

#[rstest]
#[case::one_nonempty(&["team"], Some("team"))]
#[case::two_teams(&["team-a", "team-b"], None)]
#[case::one_empty(&[""], None)]
#[case::none(&[], None)]
fn only_team_needs_one_nonempty_value(#[case] teams: &[&str], #[case] expected: Option<&str>) {
    let owned: Vec<String> = teams.iter().map(|team| (*team).to_owned()).collect();
    assert_eq!(only_team(&owned), expected.map(str::to_owned));
}

fn user(email: &str, teams: &[&str]) -> UserDetails {
    UserDetails {
        email: Some(email.to_owned()).filter(|email| !email.is_empty()),
        only_team: only_team(
            &teams
                .iter()
                .map(|team| (*team).to_owned())
                .collect::<Vec<_>>(),
        ),
    }
}

fn meta_with(user_id: Option<&str>) -> KeyMetadata {
    KeyMetadata {
        user_id: user_id.map(str::to_owned),
        ..KeyMetadata::default()
    }
}

#[test]
fn details_fill_a_falsey_email_and_a_cli_team() {
    let details = BTreeMap::from([("user".to_owned(), user("user@example.com", &["team-a"]))]);
    let merged = meta_with_user_details("cli-session-user", &meta_with(Some("user")), &details);
    assert_eq!(merged.user_email, Some("user@example.com".to_owned()));
    assert_eq!(merged.team_id, Some("team-a".to_owned()));
}

#[test]
fn details_never_infer_a_team_for_an_ordinary_key() {
    let details = BTreeMap::from([("user".to_owned(), user("user@example.com", &["team-a"]))]);
    let merged = meta_with_user_details("sk-user", &meta_with(Some("user")), &details);
    assert_eq!(merged.user_email, Some("user@example.com".to_owned()));
    assert_eq!(merged.team_id, None);
}

#[test]
fn existing_metadata_email_and_team_win_over_user_details() {
    let details = BTreeMap::from([("user".to_owned(), user("user@example.com", &["team-a"]))]);
    let meta = KeyMetadata {
        user_email: Some("meta@example.com".to_owned()),
        team_id: Some("meta-team".to_owned()),
        ..meta_with(Some("user"))
    };
    let merged = meta_with_user_details("cli-session-user", &meta, &details);
    assert_eq!(merged.user_email, Some("meta@example.com".to_owned()));
    assert_eq!(merged.team_id, Some("meta-team".to_owned()));
}

#[test]
fn empty_metadata_values_count_as_absent() {
    let details = BTreeMap::from([("user".to_owned(), user("user@example.com", &["team-a"]))]);
    let meta = KeyMetadata {
        user_email: Some(String::new()),
        team_id: Some(String::new()),
        ..meta_with(Some("user"))
    };
    let merged = meta_with_user_details("cli-session-user", &meta, &details);
    assert_eq!(merged.user_email, Some("user@example.com".to_owned()));
    assert_eq!(merged.team_id, Some("team-a".to_owned()));
}

#[rstest]
#[case::no_user_id(None, false)]
#[case::empty_user_id(Some(""), false)]
#[case::no_email(Some("user"), true)]
#[case::email_with_team(Some("user"), false)]
fn needing_details_skips_only_a_complete_ordinary_metadata(
    #[case] user_id: Option<&str>,
    #[case] needs: bool,
) {
    let meta = KeyMetadata {
        user_email: Some("meta@example.com".to_owned()),
        team_id: Some("team".to_owned()),
        ..meta_with(user_id)
    };
    let expected = if needs {
        user_id.map(str::to_owned)
    } else {
        None
    };
    assert_eq!(user_id_needing_details("sk-key", &meta), expected);
}

#[test]
fn cli_metadata_without_a_team_still_needs_user_details() {
    let meta = KeyMetadata {
        user_email: Some("meta@example.com".to_owned()),
        ..meta_with(Some("user"))
    };
    assert_eq!(
        user_id_needing_details("cli-session-user", &meta),
        Some("user".to_owned())
    );
}

#[test]
fn an_empty_metadata_email_still_needs_user_details() {
    let meta = KeyMetadata {
        user_email: Some(String::new()),
        team_id: Some("team".to_owned()),
        ..meta_with(Some("user"))
    };
    assert_eq!(
        user_id_needing_details("sk-key", &meta),
        Some("user".to_owned())
    );
}

#[test]
fn an_ordinary_key_without_a_team_never_needs_details() {
    let meta = KeyMetadata {
        user_email: Some("meta@example.com".to_owned()),
        ..meta_with(Some("user"))
    };
    assert_eq!(user_id_needing_details("sk-key", &meta), None);
}

#[test]
fn needed_user_ids_collect_each_user_once() {
    let recovered = BTreeMap::from([
        ("a".to_owned(), meta_with(Some("user"))),
        ("b".to_owned(), meta_with(Some("user"))),
        ("c".to_owned(), meta_with(None)),
    ]);
    assert_eq!(
        needed_user_ids(&recovered),
        BTreeSet::from(["user".to_owned()])
    );
}

#[test]
fn attach_details_preserves_entries_without_details() {
    let recovered = BTreeMap::from([("a".to_owned(), meta_with(Some("user")))]);
    assert_eq!(attach_details(&recovered, &BTreeMap::new()), recovered);
}

#[test]
fn token_metadata_ignores_unrequested_digests_and_keeps_the_last_row() {
    let rows = vec![
        TokenDigestRow {
            digest: "wanted".to_owned(),
            key_alias: Some("first".to_owned()),
            team_id: None,
            user_id: None,
        },
        TokenDigestRow {
            digest: "wanted".to_owned(),
            key_alias: Some("last".to_owned()),
            team_id: Some("team".to_owned()),
            user_id: Some("user".to_owned()),
        },
        TokenDigestRow {
            digest: "other".to_owned(),
            key_alias: Some("other".to_owned()),
            team_id: None,
            user_id: None,
        },
    ];
    let wanted = BTreeSet::from(["wanted".to_owned()]);
    let recovered = token_metadata(&rows, &wanted);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered["wanted"].key_alias.as_deref(), Some("last"));
    assert_eq!(recovered["wanted"].team_id.as_deref(), Some("team"));
}

#[test]
fn digest_keys_keep_only_valid_digest_text() {
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let keys = BTreeSet::from([
        digest.to_owned(),
        format!("hashed-jwt-{digest}"),
        "cli-session-user".to_owned(),
    ]);
    assert_eq!(digest_keys(&keys), BTreeSet::from([digest.to_owned()]));
}

#[test]
fn unresolved_digests_are_the_requested_ones_without_metadata() {
    let resolved = BTreeMap::from([(
        "a".to_owned(),
        KeyMetadata {
            key_alias: Some("alias".to_owned()),
            ..KeyMetadata::default()
        },
    )]);
    let digests = BTreeSet::from(["a".to_owned(), "b".to_owned()]);
    assert_eq!(
        unresolved_digests(&resolved, &digests),
        BTreeSet::from(["b".to_owned()])
    );
}

#[test]
fn daily_owners_keep_only_unanimous_requested_owners() {
    let rows = vec![
        DailyOwnerRow {
            hashed_api_key: "agree".to_owned(),
            first_owner: Some("owner".to_owned()),
            last_owner: Some("owner".to_owned()),
        },
        DailyOwnerRow {
            hashed_api_key: "single".to_owned(),
            first_owner: Some("owner".to_owned()),
            last_owner: None,
        },
        DailyOwnerRow {
            hashed_api_key: "conflict".to_owned(),
            first_owner: Some("first".to_owned()),
            last_owner: Some("last".to_owned()),
        },
        DailyOwnerRow {
            hashed_api_key: "unwanted".to_owned(),
            first_owner: Some("owner".to_owned()),
            last_owner: Some("owner".to_owned()),
        },
    ];
    let keys = BTreeSet::from([
        "agree".to_owned(),
        "single".to_owned(),
        "conflict".to_owned(),
    ]);
    assert_eq!(
        daily_owners(&rows, &keys),
        BTreeMap::from([
            ("agree".to_owned(), "owner".to_owned()),
            ("single".to_owned(), "owner".to_owned()),
        ])
    );
}

fn row(pairs: &[(&str, JsonData)]) -> BTreeMap<String, JsonData> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[rstest]
#[case::absent(None, true)]
#[case::null(Some(JsonData::Null), true)]
#[case::empty(Some(JsonData::String(String::new())), true)]
#[case::filled(Some(JsonData::String("alias".to_owned())), false)]
#[case::zero(Some(JsonData::Number(serde_json::Number::from(0))), false)]
#[case::false_flag(Some(JsonData::Bool(false)), false)]
fn export_rows_are_missing_only_for_null_or_empty_aliases(
    #[case] alias: Option<JsonData>,
    #[case] missing: bool,
) {
    let mut fields = vec![("api_key", JsonData::String("key".to_owned()))];
    fields.extend(alias.map(|alias| ("api_key_alias", alias)));
    let selected = missing_keys(&[row(&fields)], &ExportFields::default());
    assert_eq!(selected.contains("key"), missing);
}

#[test]
fn export_rows_need_a_nonempty_string_key() {
    let rows = vec![
        row(&[("api_key", JsonData::String(String::new()))]),
        row(&[("api_key", JsonData::Null)]),
        row(&[("api_key", JsonData::String("key".to_owned()))]),
    ];
    assert_eq!(
        missing_keys(&rows, &ExportFields::default()),
        BTreeSet::from(["key".to_owned()])
    );
}

fn recovered_alias(alias: Option<&str>) -> BTreeMap<String, KeyMetadata> {
    BTreeMap::from([(
        "key".to_owned(),
        KeyMetadata {
            key_alias: alias.map(str::to_owned),
            team_id: Some("recovered-team".to_owned()),
            user_email: Some("recovered@example.com".to_owned()),
            ..KeyMetadata::default()
        },
    )])
}

#[test]
fn recovered_alias_and_team_override_falsey_row_fields() {
    let rows = vec![row(&[
        ("api_key", JsonData::String("key".to_owned())),
        ("api_key_alias", JsonData::String(String::new())),
        ("team_id", JsonData::Null),
        ("user_email", JsonData::String(String::new())),
    ])];
    let projected = project_rows(
        &rows,
        &recovered_alias(Some("alias")),
        &ExportFields::default(),
    );
    assert_eq!(
        projected[0].get("api_key_alias"),
        Some(&JsonData::String("alias".to_owned()))
    );
    assert_eq!(
        projected[0].get("team_id"),
        Some(&JsonData::String("recovered-team".to_owned()))
    );
    assert_eq!(
        projected[0].get("user_email"),
        Some(&JsonData::String("recovered@example.com".to_owned()))
    );
}

#[test]
fn recovered_alias_overrides_a_filled_row_alias_but_a_filled_email_wins() {
    let rows = vec![row(&[
        ("api_key", JsonData::String("key".to_owned())),
        ("api_key_alias", JsonData::String("row-alias".to_owned())),
        ("user_email", JsonData::String("row@example.com".to_owned())),
    ])];
    let projected = project_rows(
        &rows,
        &recovered_alias(Some("alias")),
        &ExportFields::default(),
    );
    assert_eq!(
        projected[0].get("api_key_alias"),
        Some(&JsonData::String("alias".to_owned()))
    );
    assert_eq!(
        projected[0].get("user_email"),
        Some(&JsonData::String("row@example.com".to_owned()))
    );
}

#[test]
fn falsey_metadata_values_keep_the_row_value() {
    let row = row(&[
        ("api_key", JsonData::String("key".to_owned())),
        ("team_id", JsonData::String("row-team".to_owned())),
    ]);
    let projected =
        row_with_recovered_fields(&row, &recovered_alias(Some("")), &ExportFields::default());
    assert_eq!(
        projected.get("team_id"),
        Some(&JsonData::String("row-team".to_owned()))
    );
}

#[test]
fn empty_recovered_alias_and_team_keep_the_row_values() {
    let row = row(&[
        ("api_key", JsonData::String("key".to_owned())),
        ("api_key_alias", JsonData::String("row-alias".to_owned())),
        ("team_id", JsonData::String("row-team".to_owned())),
    ]);
    let recovered = BTreeMap::from([(
        "key".to_owned(),
        KeyMetadata {
            key_alias: Some(String::new()),
            team_id: Some(String::new()),
            ..KeyMetadata::default()
        },
    )]);
    let projected = row_with_recovered_fields(&row, &recovered, &ExportFields::default());
    assert_eq!(
        projected.get("api_key_alias"),
        Some(&JsonData::String("row-alias".to_owned()))
    );
    assert_eq!(
        projected.get("team_id"),
        Some(&JsonData::String("row-team".to_owned()))
    );
}

#[rstest]
#[case::false_flag(JsonData::Bool(false), false)]
#[case::true_flag(JsonData::Bool(true), true)]
#[case::zero(JsonData::Number(serde_json::Number::from(0)), false)]
#[case::nonzero(JsonData::Number(serde_json::Number::from(1)), true)]
#[case::empty_list(JsonData::Array(Vec::new()), false)]
#[case::filled_list(JsonData::Array(vec![JsonData::Null]), true)]
#[case::empty_object(JsonData::Object(BTreeMap::new()), false)]
#[case::filled_object(
    JsonData::Object(BTreeMap::from([("k".to_owned(), JsonData::Null)])),
    true
)]
#[case::empty_text(JsonData::String(String::new()), false)]
#[case::filled_text(JsonData::String("row@example.com".to_owned()), true)]
fn row_email_wins_only_when_it_is_truthy(#[case] email: JsonData, #[case] row_wins: bool) {
    let row = row(&[
        ("api_key", JsonData::String("key".to_owned())),
        ("user_email", email.clone()),
    ]);
    let recovered = BTreeMap::from([(
        "key".to_owned(),
        KeyMetadata {
            user_email: Some("recovered@example.com".to_owned()),
            ..KeyMetadata::default()
        },
    )]);
    let projected = row_with_recovered_fields(&row, &recovered, &ExportFields::default());
    let expected = if row_wins {
        email
    } else {
        JsonData::String("recovered@example.com".to_owned())
    };
    assert_eq!(projected.get("user_email"), Some(&expected));
}

#[test]
fn row_fields_are_inserted_as_null_when_neither_side_has_a_value() {
    let row = row(&[("api_key", JsonData::String("key".to_owned()))]);
    let recovered = BTreeMap::from([("key".to_owned(), meta_with(None))]);
    let projected = row_with_recovered_fields(&row, &recovered, &ExportFields::default());
    assert_eq!(projected.get("api_key_alias"), Some(&JsonData::Null));
}

#[test]
fn custom_field_names_change_only_the_projection() {
    let fields = ExportFields {
        key_column: "key".to_owned(),
        alias: "label".to_owned(),
        team_id: "team".to_owned(),
        user_email: "mail".to_owned(),
    };
    let rows = vec![row(&[
        ("key", JsonData::String("key".to_owned())),
        ("label", JsonData::String(String::new())),
        ("api_key_alias", JsonData::String("untouched".to_owned())),
    ])];
    let first = row_with_recovered_fields(&rows[0], &recovered_alias(Some("alias")), &fields);
    assert_eq!(
        first.get("label"),
        Some(&JsonData::String("alias".to_owned()))
    );
    assert_eq!(
        first.get("api_key_alias"),
        Some(&JsonData::String("untouched".to_owned()))
    );
    assert_eq!(first.get("key"), Some(&JsonData::String("key".to_owned())));
    assert_eq!(
        first.get("mail"),
        Some(&JsonData::String("recovered@example.com".to_owned()))
    );
}

#[tokio::test]
async fn filling_returns_rows_unchanged_when_nothing_is_recovered() {
    let rows = vec![row(&[
        ("api_key", JsonData::String("sk-plain".to_owned())),
        ("api_key_alias", JsonData::String(String::new())),
    ])];
    let store = PostgresStore { pool: None };
    let filled = fill_missing_api_key_aliases(&store, &rows, &ExportFields::default())
        .await
        .unwrap();
    assert_eq!(filled, rows);
}

#[test]
fn unrecovered_rows_keep_every_field_and_their_order() {
    let rows = vec![
        row(&[("api_key", JsonData::String("plain".to_owned()))]),
        row(&[("api_key", JsonData::String("key".to_owned()))]),
    ];
    let projected = project_rows(
        &rows,
        &recovered_alias(Some("alias")),
        &ExportFields::default(),
    );
    assert_eq!(projected[0], rows[0]);
    assert_eq!(
        projected[1].get("api_key_alias"),
        Some(&JsonData::String("alias".to_owned()))
    );
}

#[test]
fn user_details_skip_empty_ids_and_keep_the_last_row() {
    let first = user_table("user", Some("first@example.com"), &["team-a"]);
    let second = user_table("user", Some(""), &["team-b"]);
    let empty = user_table("", Some("empty@example.com"), &["team-c"]);
    let details = user_details(&[first, second, empty]);
    assert_eq!(details.len(), 1);
    assert_eq!(details["user"].email, None);
    assert_eq!(details["user"].only_team, Some("team-b".to_owned()));
}

fn user_table(
    user_id: &str,
    email: Option<&str>,
    teams: &[&str],
) -> crate::identity_rows::user::LiteLLMUserTable {
    serde_json::from_value(serde_json::json!({
        "user_id": user_id,
        "user_email": email,
        "teams": teams,
    }))
    .unwrap()
}

const MIGRATIONS: &str = concat!(
    include_str!("../../migrations/20250326162113_baseline/migration.sql"),
    include_str!("../../migrations/20250326171002_add_daily_user_table/migration.sql"),
    include_str!(
        "../../migrations/20251219110931_add_deleted_keys_and_deleted_teams_tables/migration.sql"
    ),
);

async fn database() -> (PostgresFixture, PostgresStore) {
    use clap::Parser as _;
    use secrecy::ExposeSecret as _;
    #[derive(clap::Parser)]
    struct Environment {
        #[arg(long, env = "POSTGRES_TEST_URL")]
        url: Option<String>,
    }
    let url = Environment::parse_from(["key-attribution-tests"])
        .url
        .map(secrecy::SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = deadpool_postgres::Manager::new(
        fixture.options.url.expose_secret().parse().unwrap(),
        tokio_postgres::NoTls,
    );
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(3)
        .build()
        .unwrap();
    pool.get()
        .await
        .unwrap()
        .batch_execute(MIGRATIONS)
        .await
        .unwrap();
    (fixture, PostgresStore { pool: Some(pool) })
}

async fn close(fixture: PostgresFixture, store: PostgresStore) {
    if let Some(pool) = store.pool {
        pool.close();
    }
    fixture.close_async().await.unwrap();
}

fn digest_of(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn timestamp(day: u32) -> chrono::NaiveDateTime {
    chrono::NaiveDate::from_ymd_opt(2026, 1, day)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
}

async fn client(store: &PostgresStore) -> deadpool_postgres::Client {
    store.pool.as_ref().unwrap().get().await.unwrap()
}

async fn insert_token(store: &PostgresStore, token: &str, alias: &str, user_id: Option<&str>) {
    client(store)
        .await
        .execute(
            r#"INSERT INTO "LiteLLM_VerificationToken" (token, key_alias, user_id) VALUES ($1, $2, $3)"#,
            &[&token, &alias, &user_id],
        )
        .await
        .unwrap();
}

async fn insert_tombstone(
    store: &PostgresStore,
    id: &str,
    token: &str,
    alias: &str,
    deleted_at: chrono::NaiveDateTime,
) {
    client(store)
        .await
        .execute(
            r#"INSERT INTO "LiteLLM_DeletedVerificationToken" (id, token, key_alias, deleted_at) VALUES ($1, $2, $3, $4)"#,
            &[&id, &token, &alias, &deleted_at],
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn postgres_active_evidence_wins_before_any_tombstone_is_read() {
    let (fixture, store) = database().await;
    insert_token(&store, "sk-active", "active-alias", Some("active-user")).await;
    insert_tombstone(&store, "d1", "sk-active", "deleted-alias", timestamp(2)).await;
    let keys = BTreeSet::from([digest_of("sk-active")]);
    let recovered = recover_double_hashed_keys(&store, &keys).await.unwrap();
    let digest = digest_of("sk-active");
    assert_eq!(
        recovered[&digest].key_alias.as_deref(),
        Some("active-alias")
    );
    assert_eq!(recovered[&digest].user_id.as_deref(), Some("active-user"));
    close(fixture, store).await;
}

#[tokio::test]
async fn postgres_deleted_lookup_takes_the_newest_tombstone_only() {
    let (fixture, store) = database().await;
    insert_tombstone(&store, "old", "sk-deleted", "old-alias", timestamp(1)).await;
    insert_tombstone(&store, "new", "sk-deleted", "new-alias", timestamp(3)).await;
    insert_tombstone(
        &store,
        "unrequested",
        "sk-other",
        "other-alias",
        timestamp(2),
    )
    .await;
    let digest = digest_of("sk-deleted");
    let keys = BTreeSet::from([digest.clone(), "not-a-digest".to_owned()]);
    let recovered = recover_double_hashed_keys(&store, &keys).await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[&digest].key_alias.as_deref(), Some("new-alias"));
    close(fixture, store).await;
}

#[tokio::test]
async fn postgres_conflicting_daily_owners_are_dropped() {
    let (fixture, store) = database().await;
    client(&store)
        .await
        .batch_execute(
            r#"INSERT INTO "LiteLLM_DailyUserSpend" (id, user_id, date, api_key, model, updated_at) VALUES
               ('1', 'owner', '2026-01-01', 'agree', 'm', '2026-01-01'),
               ('2', 'owner', '2026-01-01', 'agree', 'n', '2026-01-01'),
               ('3', 'first', '2026-01-01', 'conflict', 'm', '2026-01-01'),
               ('4', 'last', '2026-01-01', 'conflict', 'm', '2026-01-01'),
               ('5', 'owner', '2026-01-01', 'single', 'm', '2026-01-01')"#,
        )
        .await
        .unwrap();
    let keys = BTreeSet::from([
        "agree".to_owned(),
        "conflict".to_owned(),
        "single".to_owned(),
    ]);
    let owners = recover_daily_spend_owners(&store, &keys).await.unwrap();
    assert_eq!(
        owners,
        BTreeMap::from([
            ("agree".to_owned(), "owner".to_owned()),
            ("single".to_owned(), "owner".to_owned()),
        ])
    );
    close(fixture, store).await;
}

#[tokio::test]
async fn postgres_cli_keys_recover_the_existing_user_only() {
    let (fixture, store) = database().await;
    client(&store)
        .await
        .execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email, teams) VALUES ($1, $2, $3)"#,
            &[&"known", &"known@example.com", &vec!["team-a".to_owned()]],
        )
        .await
        .unwrap();
    let keys = BTreeSet::from([
        "cli-session-known".to_owned(),
        "cli-session-missing".to_owned(),
    ]);
    let recovered = recover_cli_session_keys(&store, &keys).await.unwrap();
    assert_eq!(
        recovered,
        BTreeMap::from([(
            "cli-session-known".to_owned(),
            KeyMetadata {
                key_alias: Some("cli-session-known".to_owned()),
                user_id: Some("known".to_owned()),
                ..KeyMetadata::default()
            }
        )])
    );
    close(fixture, store).await;
}

#[tokio::test]
async fn postgres_user_detail_read_binds_one_array_beyond_one_chunk() {
    let (fixture, store) = database().await;
    client(&store)
        .await
        .execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES ($1, $2)"#,
            &[&"user-5000", &"last@example.com"],
        )
        .await
        .unwrap();
    let recovered: BTreeMap<String, KeyMetadata> = (0..=5000)
        .map(|index| {
            (
                format!("key-{index}"),
                KeyMetadata {
                    user_id: Some(format!("user-{index}")),
                    ..KeyMetadata::default()
                },
            )
        })
        .collect();
    let attached = attach_user_details(&store, &recovered).await.unwrap();
    assert_eq!(
        attached["key-5000"].user_email.as_deref(),
        Some("last@example.com")
    );
    assert_eq!(attached["key-0"].user_email, None);
    close(fixture, store).await;
}

#[tokio::test]
async fn postgres_driver_failures_fall_back_to_empty_attribution() {
    let store = PostgresStore { pool: None };
    let digest = digest_of("sk-active");
    let keys = BTreeSet::from([digest]);
    assert!(
        recover_double_hashed_keys(&store, &keys)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        recover_daily_spend_owners(&store, &BTreeSet::from(["key".to_owned()]))
            .await
            .unwrap()
            .is_empty()
    );
    let recovered = BTreeMap::from([("key".to_owned(), meta_with(Some("user")))]);
    assert_eq!(
        attach_user_details(&store, &recovered).await.unwrap(),
        recovered
    );
}

#[tokio::test]
async fn postgres_malformed_user_rows_propagate() {
    let (fixture, store) = database().await;
    client(&store)
        .await
        .execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id, metadata) VALUES ($1, '[]'::jsonb)"#,
            &[&"broken"],
        )
        .await
        .unwrap();
    let recovered = BTreeMap::from([("key".to_owned(), meta_with(Some("broken")))]);
    let result = attach_user_details(&store, &recovered).await;
    assert!(matches!(result, Err(KeyAttributionError::User { .. })));
    close(fixture, store).await;
}

async fn insert_user(store: &PostgresStore, user_id: &str, email: &str, teams: Vec<String>) {
    client(store)
        .await
        .execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email, teams) VALUES ($1, $2, $3)"#,
            &[&user_id, &email, &teams],
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn postgres_export_filling_recovers_cli_and_hashed_rows_in_order() {
    let (fixture, store) = database().await;
    insert_user(
        &store,
        "cli-user",
        "cli@example.com",
        vec!["team-a".to_owned()],
    )
    .await;
    insert_token(&store, "sk-hashed", "hashed-alias", Some("hashed-user")).await;
    let rows = vec![
        row(&[
            (
                "api_key",
                JsonData::String("cli-session-cli-user".to_owned()),
            ),
            ("api_key_alias", JsonData::String(String::new())),
            ("user_email", JsonData::String(String::new())),
        ]),
        row(&[("api_key", JsonData::String(digest_of("sk-hashed")))]),
        row(&[
            ("api_key", JsonData::String("sk-plain".to_owned())),
            ("api_key_alias", JsonData::String("kept".to_owned())),
        ]),
    ];
    let projected = fill_missing_api_key_aliases(&store, &rows, &ExportFields::default())
        .await
        .unwrap();
    assert_eq!(
        projected[0].get("api_key_alias"),
        Some(&JsonData::String("cli-session-cli-user".to_owned()))
    );
    assert_eq!(
        projected[0].get("team_id"),
        Some(&JsonData::String("team-a".to_owned()))
    );
    assert_eq!(
        projected[0].get("user_email"),
        Some(&JsonData::String("cli@example.com".to_owned()))
    );
    assert_eq!(
        projected[1].get("api_key_alias"),
        Some(&JsonData::String("hashed-alias".to_owned()))
    );
    assert_eq!(projected[2], rows[2]);
    close(fixture, store).await;
}

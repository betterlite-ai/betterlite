use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use rstest::rstest;

use crate::key_attribution::history::rows::{SpendLogDigestRow, edge_metadata};
use crate::key_attribution::history::sql::{ROWS_PER_PROBE, spend_log_alias_sql};
use crate::key_attribution::history::window::{
    HistoryWindow, is_spend_log_digest, isoformat, spend_log_cache_key,
};
use crate::key_attribution::metadata::KeyMetadata;

use super::support::{DIGEST, OTHER_DIGEST, named};

fn row(digest: &str) -> SpendLogDigestRow {
    SpendLogDigestRow {
        digest: digest.to_owned(),
        first_alias: None,
        last_alias: None,
        first_team: None,
        last_team: None,
        first_owner: None,
        last_owner: None,
    }
}

#[rstest]
#[case::plain("", DIGEST, true)]
#[case::uppercase(
    "",
    "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF",
    true
)]
#[case::jwt_prefix("hashed-jwt-", DIGEST, true)]
#[case::double_jwt_prefix("hashed-jwt-hashed-jwt-", DIGEST, false)]
#[case::shouting_prefix("HASHED-JWT-", DIGEST, false)]
#[case::prefix_alone("hashed-jwt-", "", false)]
#[case::short("", "0123456789abcdef", false)]
#[case::one_character_too_long(
    "",
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0",
    false
)]
#[case::not_hex(
    "",
    "g123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    false
)]
#[case::other_prefix("sk-", DIGEST, false)]
fn only_a_digest_or_one_jwt_prefixed_digest_is_probed(
    #[case] prefix: &str,
    #[case] body: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_spend_log_digest(&format!("{prefix}{body}")), expected);
}

#[rstest]
#[case::whole_seconds(0, "2026-01-01T00:00:00")]
#[case::microseconds(123_456, "2026-01-01T00:00:00.123456")]
#[case::one_microsecond(1, "2026-01-01T00:00:00.000001")]
fn timestamps_print_like_python_isoformat(#[case] micros: u32, #[case] expected: &str) {
    let value = NaiveDate::from_ymd_opt(2026, 1, 1)
        .unwrap()
        .and_hms_micro_opt(0, 0, 0, micros)
        .unwrap();
    assert_eq!(isoformat(value), expected);
}

#[test]
fn cache_key_keeps_the_original_digest_and_both_window_ends() {
    let prefixed = format!("hashed-jwt-{DIGEST}");
    let window = HistoryWindow {
        start: NaiveDate::from_ymd_opt(2026, 3, 4)
            .unwrap()
            .and_hms_micro_opt(5, 6, 7, 8)
            .unwrap(),
        end: NaiveDate::from_ymd_opt(2026, 3, 5)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
    };
    assert_eq!(
        spend_log_cache_key(&prefixed, &window),
        format!("spend_log_key_metadata:{prefixed}:2026-03-04T05:06:07.000008:2026-03-05T00:00:00")
    );
}

#[rstest]
#[case::both_absent(None, None, None)]
#[case::first_only(Some("a"), None, Some("a"))]
#[case::last_only(None, Some("a"), Some("a"))]
#[case::agreeing(Some("a"), Some("a"), Some("a"))]
#[case::conflicting(Some("a"), Some("b"), None)]
fn each_field_keeps_only_unanimous_values(
    #[case] first: Option<&str>,
    #[case] last: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let first_value = first.map(str::to_owned);
    let last_value = last.map(str::to_owned);
    let wanted = expected.map(str::to_owned);
    let alias = SpendLogDigestRow {
        first_alias: first_value.clone(),
        last_alias: last_value.clone(),
        ..row(DIGEST)
    };
    let team = SpendLogDigestRow {
        first_team: first_value.clone(),
        last_team: last_value.clone(),
        ..row(DIGEST)
    };
    let owner = SpendLogDigestRow {
        first_owner: first_value,
        last_owner: last_value,
        ..row(DIGEST)
    };
    assert_eq!(alias.metadata().key_alias, wanted);
    assert_eq!(alias.metadata().team_id, None);
    assert_eq!(alias.metadata().user_id, None);
    assert_eq!(team.metadata().team_id, wanted);
    assert_eq!(team.metadata().key_alias, None);
    assert_eq!(owner.metadata().user_id, wanted);
    assert_eq!(owner.metadata().team_id, None);
}

#[test]
fn a_disagreeing_field_does_not_drop_its_neighbours() {
    let mixed = SpendLogDigestRow {
        first_alias: Some("old".to_owned()),
        last_alias: Some("new".to_owned()),
        first_team: Some("team".to_owned()),
        last_team: Some("team".to_owned()),
        first_owner: None,
        last_owner: Some("owner".to_owned()),
        ..row(DIGEST)
    };
    assert_eq!(
        mixed.metadata(),
        KeyMetadata {
            key_alias: None,
            team_id: Some("team".to_owned()),
            user_id: Some("owner".to_owned()),
            user_email: None,
            key_exists: None,
        }
    );
}

#[test]
fn edge_metadata_drops_unrequested_digests_and_rows_without_a_value() {
    let kept = SpendLogDigestRow {
        first_alias: Some("kept".to_owned()),
        ..row(DIGEST)
    };
    let unrequested = SpendLogDigestRow {
        first_alias: Some("stranger".to_owned()),
        ..row("unrequested")
    };
    let conflicting = SpendLogDigestRow {
        first_alias: Some("a".to_owned()),
        last_alias: Some("b".to_owned()),
        ..row(OTHER_DIGEST)
    };
    let empty_text = SpendLogDigestRow {
        first_alias: Some(String::new()),
        last_alias: Some(String::new()),
        ..row("empty")
    };
    let nothing = row("nothing");
    let requested = BTreeSet::from([
        DIGEST.to_owned(),
        OTHER_DIGEST.to_owned(),
        "empty".to_owned(),
        "nothing".to_owned(),
    ]);
    let rows = [kept, unrequested, conflicting, empty_text, nothing];
    assert_eq!(
        edge_metadata(&rows, &requested),
        BTreeMap::from([(DIGEST.to_owned(), named("kept"))])
    );
}

#[test]
fn edge_metadata_keeps_a_row_whose_only_value_is_the_team() {
    let team_only = SpendLogDigestRow {
        last_team: Some("team".to_owned()),
        ..row(DIGEST)
    };
    let requested = BTreeSet::from([DIGEST.to_owned()]);
    let recovered = edge_metadata(&[team_only], &requested);
    assert_eq!(recovered[DIGEST].team_id.as_deref(), Some("team"));
}

#[test]
fn the_probe_sql_binds_one_array_and_both_window_ends_and_caps_both_edges() {
    let sql = spend_log_alias_sql();
    assert_eq!(ROWS_PER_PROBE, 100);
    assert_eq!(sql.matches("LIMIT 100").count(), 2);
    assert_eq!(sql.matches("OFFSET 99").count(), 1);
    assert!(sql.contains("unnest($1::text[]) AS keys(digest)"));
    assert!(sql.contains(r#""startTime" >= oldest_probe.stopped_at"#));
    assert!(sql.contains(r#""startTime" < $3::timestamp"#));
}

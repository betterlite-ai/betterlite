use std::path::PathBuf;

use rstest::rstest;

use crate::spend_rows::key::{
    CounterKeyError, SpendCounterKey, WindowEntity, decode_spend_counter_key,
};
use crate::spend_rows::tests::support::{FixedDurations, replay_corpus_key};

#[rstest]
#[case::key("spend:key:abc", SpendCounterKey::Key("abc"))]
#[case::team_member(
    "spend:team_member:u1:t1",
    SpendCounterKey::TeamMember {
        user_id: "u1",
        team_id: "t1",
    }
)]
#[case::team("spend:team:t1", SpendCounterKey::Team("t1"))]
#[case::user("spend:user:u1", SpendCounterKey::User("u1"))]
#[case::organization("spend:org:o1", SpendCounterKey::Organization("o1"))]
#[case::project("spend:project:p1", SpendCounterKey::Project("p1"))]
#[case::end_user("spend:end_user:e1", SpendCounterKey::EndUser("e1"))]
#[case::unknown("spend:model:m1", SpendCounterKey::Unknown("spend:model:m1"))]
#[case::team_members_is_not_a_member(
    "spend:team_members:x",
    SpendCounterKey::Unknown("spend:team_members:x")
)]
fn decodes_every_counter_prefix(
    #[case] counter_key: &str,
    #[case] expected: SpendCounterKey<'static>,
) {
    let decoded = decode_spend_counter_key(counter_key.as_bytes(), &FixedDurations)
        .expect("a decoded counter key");
    assert_eq!(decoded, expected);
}

#[rstest]
#[case::key_window(
    "spend:key:abc:window:1h",
    SpendCounterKey::Window {
        entity: WindowEntity::Key,
        duration: "1h",
    }
)]
#[case::team_window(
    "spend:team:t1:window:30d",
    SpendCounterKey::Window {
        entity: WindowEntity::Team,
        duration: "30d",
    }
)]
#[case::last_window_segment_wins(
    "spend:key:a:window:1h:window:2d",
    SpendCounterKey::Window {
        entity: WindowEntity::Key,
        duration: "2d",
    }
)]
#[case::invalid_duration_stays_literal(
    "spend:key:abc:window:soon",
    SpendCounterKey::Key("abc:window:soon")
)]
#[case::empty_duration_stays_literal("spend:key:abc:window:", SpendCounterKey::Key("abc:window:"))]
#[case::only_key_and_team_windows("spend:user:u1:window:1h", SpendCounterKey::User("u1:window:1h"))]
fn window_counters_need_a_nonempty_valid_duration(
    #[case] counter_key: &str,
    #[case] expected: SpendCounterKey<'static>,
) {
    let decoded = decode_spend_counter_key(counter_key.as_bytes(), &FixedDurations)
        .expect("a decoded counter key");
    assert_eq!(decoded, expected);
}

#[rstest]
#[case::last_colon_separates(
    "spend:team_member:user:with:colon:team-1",
    "user:with:colon",
    "team-1"
)]
#[case::empty_team_stays_literal("spend:team_member:user:", "user", "")]
#[case::window_suffix_is_part_of_the_id("spend:team_member:u:window:1h", "u:window", "1h")]
fn team_member_splits_at_the_last_colon(
    #[case] counter_key: &str,
    #[case] user_id: &str,
    #[case] team_id: &str,
) {
    let decoded = decode_spend_counter_key(counter_key.as_bytes(), &FixedDurations)
        .expect("a decoded counter key");
    assert_eq!(decoded, SpendCounterKey::TeamMember { user_id, team_id });
}

#[test]
fn a_member_suffix_without_a_colon_is_unknown() {
    let decoded = decode_spend_counter_key(b"spend:team_member:user-only", &FixedDurations)
        .expect("a decoded counter key");
    assert_eq!(
        decoded,
        SpendCounterKey::Unknown("spend:team_member:user-only")
    );
}

#[test]
fn retains_a_long_id_literally() {
    let identifier = "x".repeat(4096);
    let counter_key = format!("spend:key:{identifier}");
    let decoded = decode_spend_counter_key(counter_key.as_bytes(), &FixedDurations)
        .expect("a decoded counter key");
    assert_eq!(decoded, SpendCounterKey::Key(identifier.as_str()));
}

#[test]
fn rejects_key_bytes_that_are_not_text() {
    let bytes = [0xff_u8, 0xfe, 0x00];
    let decoded = decode_spend_counter_key(&bytes, &FixedDurations);
    assert!(matches!(decoded, Err(CounterKeyError::NotText(_error))));
}

#[rstest]
fn replays_the_spend_counter_key_corpus(#[files("fuzz/corpus/spend_counter_key/*")] path: PathBuf) {
    let bytes = std::fs::read(&path).expect("a corpus file");
    let stem = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .expect("a corpus file name");
    assert_eq!(replay_corpus_key(&bytes), expected_corpus_key(stem));
}

fn expected_corpus_key(stem: &str) -> String {
    match stem {
        "key-plain" => "key:abc".to_owned(),
        "team-window" => "window:team:1d".to_owned(),
        "ambiguous-window" => "window:key:2d".to_owned(),
        "empty-window-duration" => "key:a:window:".to_owned(),
        "member-last-colon" => "team_member:user:with:colon:team-1".to_owned(),
        "member-without-colon" => "unknown:spend:team_member:user-only".to_owned(),
        "long-id" => format!("key:{}", "x".repeat(512)),
        "not-utf8" => "error".to_owned(),
        _other => panic!("unexpected corpus file {stem}"),
    }
}

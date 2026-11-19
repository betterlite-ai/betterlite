use betterlite_db::budget_reset_policy::cache_invalidation::GLOBAL_PROXY_SPEND_CACHE_KEY;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use super::super::cache_read_keys;

#[test]
fn no_response_cost_returns_no_keys() {
    let tags = [Json::Text("qa".to_owned())];
    let keys = cache_read_keys(
        Some("user-1"),
        Some("end-1"),
        Some("team-1"),
        Some(&tags),
        None,
    );
    assert!(keys.is_empty());
}

#[test]
fn the_user_and_the_global_spend_come_first() {
    let keys = cache_read_keys(Some("user-1"), None, None, None, Some(Decimal::from(1_i32)));
    assert_eq!(keys, ["user-1", GLOBAL_PROXY_SPEND_CACHE_KEY]);
}

#[test]
fn no_user_means_no_user_and_no_global_key() {
    let keys = cache_read_keys(
        None,
        Some("end-1"),
        Some("team-1"),
        None,
        Some(Decimal::from(1_i32)),
    );
    assert_eq!(keys, ["end_user_id:end-1", "team_id:team-1"]);
}

#[test]
fn end_user_and_team_follow_the_user_keys() {
    let keys = cache_read_keys(
        Some("user-1"),
        Some("end-1"),
        Some("team-1"),
        None,
        Some(Decimal::from(1_i32)),
    );
    assert_eq!(
        keys,
        [
            "user-1",
            GLOBAL_PROXY_SPEND_CACHE_KEY,
            "end_user_id:end-1",
            "team_id:team-1",
        ]
    );
}

#[test]
fn tags_keep_only_non_empty_text_in_order() {
    let tags = [
        Json::Text("qa".to_owned()),
        Json::Text(String::new()),
        Json::Number(serde_json::Number::from(7_i32)),
        Json::Null,
        Json::Text("prod".to_owned()),
    ];
    let keys = cache_read_keys(None, None, None, Some(&tags), Some(Decimal::from(1_i32)));
    assert_eq!(keys, ["tag:qa", "tag:prod"]);
}

#[test]
fn empty_ids_still_produce_keys_and_a_zero_cost_still_counts() {
    let keys = cache_read_keys(Some(""), Some(""), Some(""), None, Some(Decimal::ZERO));
    assert_eq!(
        keys,
        ["", GLOBAL_PROXY_SPEND_CACHE_KEY, "end_user_id:", "team_id:"]
    );
}

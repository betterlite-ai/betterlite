#![cfg(test)]

mod postgres;
mod support;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::budget_permission_rows::JsonData;

use self::support::HalfJitter;
use super::dispatch::{QueryType, TableName, UpdateSelector, UpdateTarget};
use super::hash::{hash_token, hash_token_if_needed};
use super::json_text::{FAILED_TO_SERIALIZE_JSON, JsonEncoder, SerdeJsonEncoder, jsonify_object};
use super::retry::{MAX_TIME, MAX_TRIES, SystemWriteClock, retry_wait};

#[test]
fn hash_utf8_lower_hex() {
    assert_eq!(
        hash_token(""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hash_token("sk-abc"),
        "1460db1b6902f8b1fc2a40d9381a24d0fd22c3bc1b2c6f999c521da73776fbe0"
    );
    assert_eq!(
        hash_token("\u{043a}\u{043b}\u{044e}\u{0447}"),
        "1de36a32af798da0c1ac9297603a320ed8fe567cf21c9177112a4ce914ebb8be"
    );
    assert_eq!(
        hash_token("SK-ABC"),
        "6e41f05540b7da82af5fbdffd88cf7ef62e774b1bf37ab8a045562077d9086e9"
    );
}

#[test]
fn insert_always_hashes() {
    let stored = hash_token("sk-abc");
    assert_ne!(stored, "sk-abc");
    assert_eq!(
        hash_token(&stored),
        "125608c42b7910775fab830f88ae315427bfbb945e21af384f19f28d77918bfe"
    );
}

#[test]
fn conditional_hash_prefix() {
    assert_eq!(
        hash_token_if_needed("sk-abc"),
        "1460db1b6902f8b1fc2a40d9381a24d0fd22c3bc1b2c6f999c521da73776fbe0"
    );
    assert_eq!(
        hash_token_if_needed("sk-"),
        "449b11820b0f65268bca74c8c9153fa9bebd2a9ee8b3e6ec479d768b46242c2a"
    );
    assert_eq!(hash_token_if_needed("abc"), "abc");
    assert_eq!(hash_token_if_needed("SK-ABC"), "SK-ABC");
    assert_eq!(
        hash_token_if_needed("1460db1b6902f8b1fc2a40d9381a24d0fd22c3bc1b2c6f999c521da73776fbe0"),
        "1460db1b6902f8b1fc2a40d9381a24d0fd22c3bc1b2c6f999c521da73776fbe0"
    );
    assert_eq!(hash_token_if_needed(""), "");
}

fn nested_object() -> JsonData {
    JsonData::Object(BTreeMap::from([
        (
            String::from("a"),
            JsonData::Number(serde_json::Number::from(1)),
        ),
        (String::from("b"), JsonData::String(String::from("x"))),
    ]))
}

#[test]
fn json_dict_only_copy() {
    let input = BTreeMap::from([
        (String::from("nested"), nested_object()),
        (
            String::from("text"),
            JsonData::String(String::from("{\"a\":1}")),
        ),
    ]);
    let converted = jsonify_object(&input, &SerdeJsonEncoder);
    assert_eq!(
        converted.get("nested"),
        Some(&JsonData::String(String::from("{\"a\":1,\"b\":\"x\"}")))
    );
    assert_eq!(
        converted.get("text"),
        Some(&JsonData::String(String::from("{\"a\":1}")))
    );
    assert_eq!(input.get("nested"), Some(&nested_object()));
}

#[test]
fn json_non_dict_values_untouched() {
    let listed = JsonData::Array(vec![nested_object()]);
    let input = BTreeMap::from([
        (String::from("listed"), listed.clone()),
        (String::from("null"), JsonData::Null),
        (String::from("flag"), JsonData::Bool(true)),
        (
            String::from("number"),
            JsonData::Number(serde_json::Number::from(7)),
        ),
    ]);
    let converted = jsonify_object(&input, &SerdeJsonEncoder);
    assert_eq!(converted.get("listed"), Some(&listed));
    assert_eq!(converted.get("null"), Some(&JsonData::Null));
    assert_eq!(converted.get("flag"), Some(&JsonData::Bool(true)));
    assert_eq!(
        converted.get("number"),
        Some(&JsonData::Number(serde_json::Number::from(7)))
    );
}

struct FailingEncoder;

impl JsonEncoder for FailingEncoder {
    fn encode(&self, _value: &JsonData) -> Result<String, serde_json::Error> {
        Err(serde_json::from_str::<bool>("not-json").unwrap_err())
    }
}

#[test]
fn json_encode_failure_marker() {
    assert_eq!(FAILED_TO_SERIALIZE_JSON, "failed-to-serialize-json");
    let input = BTreeMap::from([(
        String::from("metadata"),
        JsonData::Object(BTreeMap::from([(
            String::from("team"),
            JsonData::String(String::from("acme")),
        )])),
    )]);
    let converted = jsonify_object(&input, &FailingEncoder);
    assert_eq!(
        converted.get("metadata"),
        Some(&JsonData::String(String::from(FAILED_TO_SERIALIZE_JSON)))
    );
}

#[test]
fn system_clock_reads_wall_time() {
    let clock = SystemWriteClock;
    assert!(clock.now().timestamp() > 1_600_000_000);
    assert!(clock.instant() <= Instant::now());
}

#[test]
fn retry_wait_is_jittered_expo_capped_by_remaining_time() {
    assert_eq!(MAX_TRIES, 3);
    assert_eq!(MAX_TIME, Duration::from_secs(10));
    assert_eq!(
        retry_wait(&HalfJitter, 1, Duration::ZERO),
        Duration::from_millis(500)
    );
    assert_eq!(
        retry_wait(&HalfJitter, 2, Duration::ZERO),
        Duration::from_secs(1)
    );
    assert_eq!(
        retry_wait(&HalfJitter, 3, Duration::ZERO),
        Duration::from_secs(2)
    );
    assert_eq!(
        retry_wait(&HalfJitter, 3, Duration::from_secs(3)),
        Duration::from_secs(2)
    );
    assert_eq!(
        retry_wait(&HalfJitter, 6, Duration::from_secs(3)),
        Duration::from_secs(7)
    );
    assert_eq!(retry_wait(&HalfJitter, 1, MAX_TIME), Duration::ZERO);
}

#[test]
fn update_selector_matches_update_data_branches() {
    let key = UpdateSelector {
        token: Some("sk-abc"),
        user_id: Some("user-a"),
        table_name: Some(TableName::User),
        query_type: QueryType::UpdateMany,
    };
    assert_eq!(key.target(), UpdateTarget::Key);
    let user_by_id = UpdateSelector {
        token: None,
        user_id: Some("user-a"),
        table_name: None,
        query_type: QueryType::UpdateMany,
    };
    assert_eq!(user_by_id.target(), UpdateTarget::User);
    let user_by_table = UpdateSelector {
        token: None,
        user_id: None,
        table_name: Some(TableName::User),
        query_type: QueryType::Update,
    };
    assert_eq!(user_by_table.target(), UpdateTarget::User);
    let user_many = UpdateSelector {
        token: None,
        user_id: None,
        table_name: Some(TableName::User),
        query_type: QueryType::UpdateMany,
    };
    assert_eq!(user_many.target(), UpdateTarget::Elsewhere);
    let key_table_without_token = UpdateSelector {
        token: None,
        user_id: None,
        table_name: Some(TableName::Key),
        query_type: QueryType::Update,
    };
    assert_eq!(key_table_without_token.target(), UpdateTarget::Elsewhere);
}

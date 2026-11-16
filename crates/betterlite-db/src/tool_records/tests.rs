#![cfg(test)]

mod postgres;
mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use chrono::NaiveDateTime;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::budget_permission_rows::{JsonData, Timestamp};
use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::resource_rows::ResourceRowsError;

use super::decode::{ToolRecordDecodeError, decode_tool_record};
use super::owners::{attach_owners, owners_from_rows};
use super::records::{
    LiteLLMToolTableRow, ToolCallPolicy, ToolDetailResponse, ToolDiscoveryUser, ToolInputPolicy,
    ToolListResponse, ToolOutputPolicy, ToolPolicyOption, ToolPolicyOptionsResponse,
    ToolPolicyOverrideRow, ToolPolicyUpdateRequest, ToolPolicyUpdateResponse, ToolSpendResponse,
    ToolUsageLogsResponse,
};

fn tool(text: &str) -> LiteLLMToolTableRow {
    decode_tool_record(text.as_bytes()).unwrap()
}

fn token(text: &str) -> LiteLLMVerificationToken {
    serde_json::from_str(text).unwrap()
}

fn user(text: &str) -> LiteLLMUserTable {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::trusted(ToolCallPolicy::Trusted, "trusted")]
#[case::untrusted(ToolCallPolicy::Untrusted, "untrusted")]
#[case::dual_llm(ToolCallPolicy::DualLlm, "dual_llm")]
#[case::blocked(ToolCallPolicy::Blocked, "blocked")]
fn call_policies_use_the_litellm_literals(#[case] policy: ToolCallPolicy, #[case] text: &str) {
    assert_eq!(
        serde_json::to_value(policy).unwrap(),
        serde_json::json!(text)
    );
    assert_eq!(
        serde_json::from_value::<ToolCallPolicy>(serde_json::json!(text)).unwrap(),
        policy
    );
    assert!(serde_json::from_value::<ToolCallPolicy>(serde_json::json!("guardrail")).is_err());
}

#[rstest]
#[case::absent("{}", ToolInputPolicy::Untrusted, 0)]
#[case::null(
    r#"{"input_policy":null,"call_count":null}"#,
    ToolInputPolicy::Untrusted,
    0
)]
#[case::empty_string(
    r#"{"input_policy":"","call_count":""}"#,
    ToolInputPolicy::Untrusted,
    0
)]
#[case::zero_and_false(
    r#"{"input_policy":0,"call_count":false}"#,
    ToolInputPolicy::Untrusted,
    0
)]
#[case::zero_count(r#"{"call_count":0}"#, ToolInputPolicy::Untrusted, 0)]
#[case::empty_containers(
    r#"{"input_policy":[],"call_count":{}}"#,
    ToolInputPolicy::Untrusted,
    0
)]
#[case::explicit_policy(
    r#"{"input_policy":"blocked","call_count":7}"#,
    ToolInputPolicy::Blocked,
    7
)]
#[case::negative_count(
    r#"{"input_policy":"trusted","call_count":-3}"#,
    ToolInputPolicy::Trusted,
    -3
)]
fn falsy_input_policies_and_counts_take_their_defaults(
    #[case] fields: &str,
    #[case] policy: ToolInputPolicy,
    #[case] call_count: i64,
) {
    let row = tool(fields);
    assert_eq!(row.input_policy, policy);
    assert_eq!(row.call_count, call_count);
}

#[rstest]
#[case::absent("{}", ToolOutputPolicy::Untrusted)]
#[case::null(r#"{"output_policy":null}"#, ToolOutputPolicy::Untrusted)]
#[case::empty_string(r#"{"output_policy":""}"#, ToolOutputPolicy::Untrusted)]
#[case::falsy_zero(r#"{"output_policy":0}"#, ToolOutputPolicy::Untrusted)]
#[case::explicit_trust(r#"{"output_policy":"trusted"}"#, ToolOutputPolicy::Trusted)]
fn falsy_output_policies_take_untrusted(#[case] fields: &str, #[case] policy: ToolOutputPolicy) {
    assert_eq!(tool(fields).output_policy, policy);
}

#[test]
fn null_discovery_columns_stay_null() {
    let row = tool(
        r#"{"tool_id":"t","tool_name":"n","origin":null,"key_hash":null,"team_id":null,"key_alias":null,"user_agent":null,"assignments":null,"last_used_at":null,"created_at":null,"updated_at":null,"created_by":null,"updated_by":null}"#,
    );
    assert_eq!(row.origin, None);
    assert_eq!(row.key_hash, None);
    assert_eq!(row.team_id, None);
    assert_eq!(row.key_alias, None);
    assert_eq!(row.user_agent, None);
    assert_eq!(row.assignments, None);
    assert_eq!(row.last_used_at, None);
    assert_eq!(row.created_at, None);
    assert_eq!(row.updated_at, None);
    assert_eq!(row.created_by, None);
    assert_eq!(row.updated_by, None);
    assert_eq!(row.user, None);
}

#[test]
fn discovery_columns_timestamps_and_assignments_survive_the_conversion() {
    let row = tool(
        r#"{"tool_id":"t1","tool_name":"echo","origin":"user_defined","key_hash":"k1","team_id":"tm","key_alias":"alias","user_agent":"agent/1","assignments":{"scope":"read"},"created_at":"2026-01-02T03:04:05","last_used_at":"2026-01-03T04:05:06","created_by":"creator","updated_by":"updater"}"#,
    );
    assert_eq!(row.tool_id, "t1");
    assert_eq!(row.tool_name, "echo");
    assert_eq!(row.origin.as_deref(), Some("user_defined"));
    assert_eq!(row.key_hash.as_deref(), Some("k1"));
    assert_eq!(row.team_id.as_deref(), Some("tm"));
    assert_eq!(row.key_alias.as_deref(), Some("alias"));
    assert_eq!(row.user_agent.as_deref(), Some("agent/1"));
    let assignments: BTreeMap<String, JsonData> =
        [("scope".to_owned(), JsonData::String("read".to_owned()))]
            .into_iter()
            .collect();
    assert_eq!(row.assignments, Some(assignments));
    let created =
        NaiveDateTime::parse_from_str("2026-01-02T03:04:05", "%Y-%m-%dT%H:%M:%S").unwrap();
    assert_eq!(row.created_at, Some(Timestamp::Naive(created)));
    let used = NaiveDateTime::parse_from_str("2026-01-03T04:05:06", "%Y-%m-%dT%H:%M:%S").unwrap();
    assert_eq!(row.last_used_at, Some(Timestamp::Naive(used)));
    assert_eq!(row.created_by.as_deref(), Some("creator"));
    assert_eq!(row.updated_by.as_deref(), Some("updater"));
}

#[test]
fn absent_identifiers_default_to_empty_strings() {
    let row = tool(r#"{"input_policy":"trusted"}"#);
    assert_eq!(row.tool_id, "");
    assert_eq!(row.tool_name, "");
}

#[test]
fn spend_records_keep_decimal_amounts() {
    let spend: ToolSpendResponse = serde_json::from_str(
        r#"{"by_tool":[{"tool_name":"echo","spend":1.234567890123456789,"call_count":2,"total_tokens":10}],"daily":[{"date":"2026-01-02","tool_name":"echo","spend":0.0000001,"call_count":1}]}"#,
    )
    .unwrap();
    assert_eq!(
        spend.by_tool[0].spend,
        "1.234567890123456789".parse::<Decimal>().unwrap()
    );
    assert_eq!(
        spend.daily[0].spend,
        "0.0000001".parse::<Decimal>().unwrap()
    );
    let logs: ToolUsageLogsResponse = serde_json::from_str(
        r#"{"logs":[{"id":"r1","timestamp":"2026-01-02T03:04:05","spend":0.5},{"id":"r2","timestamp":"2026-01-02T03:04:06","spend":null}],"total":2,"page":1,"page_size":10}"#,
    )
    .unwrap();
    assert_eq!(logs.logs[0].spend, Some("0.5".parse::<Decimal>().unwrap()));
    assert_eq!(logs.logs[1].spend, None);
}

#[test]
fn declared_records_take_the_fields_litellm_declares() {
    let update: ToolPolicyUpdateRequest = serde_json::from_str(r#"{"tool_name":"echo"}"#).unwrap();
    assert_eq!(update.tool_name, "echo");
    assert_eq!(update.input_policy, None);
    assert_eq!(update.output_policy, None);
    assert_eq!(update.team_id, None);
    assert_eq!(update.key_hash, None);
    assert_eq!(update.key_alias, None);
    let answer: ToolPolicyUpdateResponse =
        serde_json::from_str(r#"{"tool_name":"echo","updated":true}"#).unwrap();
    assert!(answer.updated);
    assert_eq!(answer.input_policy, None);
    let detail: ToolDetailResponse =
        serde_json::from_str(r#"{"tool":{"tool_id":"t","tool_name":"n"}}"#).unwrap();
    assert_eq!(detail.tool.tool_id, "t");
    assert!(detail.overrides.is_empty());
    let list: ToolListResponse = serde_json::from_str(r#"{"tools":[],"total":0}"#).unwrap();
    assert_eq!(list.total, 0);
    let options: ToolPolicyOptionsResponse =
        serde_json::from_str(r#"{"input_policies":[],"output_policies":[]}"#).unwrap();
    assert!(options.input_policies.is_empty());
    let option: ToolPolicyOption =
        serde_json::from_str(r#"{"value":"trusted","label":"Trusted","description":"allows"}"#)
            .unwrap();
    assert_eq!(option.label, "Trusted");
    let spend: ToolSpendResponse = serde_json::from_str("{}").unwrap();
    assert!(spend.by_tool.is_empty());
    assert!(spend.daily.is_empty());
    assert_eq!(spend.start_date, None);
    assert_eq!(spend.end_date, None);
    let override_row: ToolPolicyOverrideRow =
        serde_json::from_str(r#"{"override_id":"perm-1","tool_name":"echo"}"#).unwrap();
    assert_eq!(override_row.input_policy, ToolInputPolicy::Blocked);
    assert_eq!(override_row.key_alias, None);
    assert_eq!(override_row.created_at, None);
}

#[test]
fn override_records_serialize_every_field_like_litellm() {
    let row = ToolPolicyOverrideRow {
        override_id: "perm-1".to_owned(),
        tool_name: "echo".to_owned(),
        team_id: None,
        key_hash: Some("key-1".to_owned()),
        input_policy: ToolInputPolicy::Blocked,
        key_alias: None,
        created_at: None,
        updated_at: None,
    };
    assert_eq!(
        serde_json::to_value(row).unwrap(),
        serde_json::json!({
            "override_id": "perm-1",
            "tool_name": "echo",
            "team_id": null,
            "key_hash": "key-1",
            "input_policy": "blocked",
            "key_alias": null,
            "created_at": null,
            "updated_at": null,
        })
    );
}

#[test]
fn an_owner_column_in_the_stored_row_is_ignored() {
    let row = tool(r#"{"tool_id":"t","tool_name":"n","user":{"user_id":"u1"}}"#);
    assert_eq!(row.user, None);
}

#[test]
fn unknown_columns_are_ignored() {
    let row = tool(r#"{"tool_id":"t","tool_name":"n","object_permission_id":"p"}"#);
    assert_eq!(row.tool_id, "t");
}

#[rstest]
#[case::array("[]")]
#[case::scalar("5")]
#[case::string(r#""row""#)]
fn a_stored_row_that_is_not_an_object_is_rejected(#[case] text: &str) {
    assert!(matches!(
        decode_tool_record(text.as_bytes()),
        Err(ToolRecordDecodeError::NotAnObject)
    ));
}

#[rstest]
#[case::invalid_utf8(b"\xff\xfe")]
#[case::duplicate_keys(br#"{"tool_id":"a","tool_id":"b"}"#)]
#[case::truncated_json(br#"{"tool_id":"#)]
fn a_row_that_cannot_be_parsed_is_rejected(#[case] bytes: &[u8]) {
    assert!(matches!(
        decode_tool_record(bytes),
        Err(ToolRecordDecodeError::Row { .. })
    ));
}

#[test]
fn a_row_over_the_size_limit_is_rejected() {
    let text = format!(r#"{{"tool_id":"{}"}}"#, "a".repeat(0x0010_0000));
    assert!(matches!(
        decode_tool_record(text.as_bytes()),
        Err(ToolRecordDecodeError::Row {
            source: ResourceRowsError::TooLarge { .. }
        })
    ));
}

#[rstest]
#[case::string_id(r#"{"tool_id":5}"#)]
#[case::unknown_input_policy(r#"{"input_policy":"dual_llm"}"#)]
#[case::unknown_output_policy(r#"{"output_policy":"blocked"}"#)]
#[case::string_call_count(r#"{"call_count":"7"}"#)]
#[case::array_assignments(r#"{"assignments":[]}"#)]
fn a_row_that_does_not_match_the_record_is_rejected(#[case] text: &str) {
    assert!(matches!(
        decode_tool_record(text.as_bytes()),
        Err(ToolRecordDecodeError::Model { .. })
    ));
}

#[test]
fn owners_map_each_key_to_the_user_that_exists() {
    let tokens = [
        token(r#"{"token":"k1","user_id":"u1"}"#),
        token(r#"{"token":"k2","user_id":"u2"}"#),
        token(r#"{"token":"k3","user_id":null}"#),
        token(r#"{"token":null,"user_id":"u1"}"#),
        token(r#"{"token":"","user_id":"u1"}"#),
        token(r#"{"token":"k4","user_id":""}"#),
    ];
    let users = [user(
        r#"{"user_id":"u1","user_email":"u1@example.com","user_alias":"one"}"#,
    )];
    let owners = owners_from_rows(&tokens, &users);
    assert_eq!(
        owners,
        [(
            "k1".to_owned(),
            ToolDiscoveryUser {
                user_id: "u1".to_owned(),
                user_email: Some("u1@example.com".to_owned()),
                user_alias: Some("one".to_owned()),
            }
        )]
        .into_iter()
        .collect()
    );
}

#[test]
fn attach_owners_leaves_tools_without_a_known_owner_null() {
    let owners = owners_from_rows(
        &[token(r#"{"token":"k1","user_id":"u1"}"#)],
        &[user(r#"{"user_id":"u1"}"#)],
    );
    let mut tools = vec![
        tool(r#"{"tool_id":"a","key_hash":"k1"}"#),
        tool(r#"{"tool_id":"b","key_hash":"other"}"#),
        tool(r#"{"tool_id":"c","key_hash":null}"#),
    ];
    attach_owners(&mut tools, &owners);
    assert_eq!(
        tools[0].user,
        Some(ToolDiscoveryUser {
            user_id: "u1".to_owned(),
            user_email: None,
            user_alias: None,
        })
    );
    assert_eq!(tools[1].user, None);
    assert_eq!(tools[2].user, None);
}

#[rstest]
#[files("fuzz/corpus/tool_record/*")]
fn replays_the_tool_record_corpus(path: PathBuf) {
    drop(decode_tool_record(&fs::read(path).unwrap()));
}

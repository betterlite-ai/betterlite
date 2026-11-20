use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::daily_spend_sql::values::SpendColumnValue;
use crate::redis_spend::codec::{
    MAX_SPEND_BUFFER_BYTES, SpendBufferError, SpendBufferKind, SpendBufferRecord,
    combine_entity_transactions, decode_daily_batch, decode_entity_batch, decode_spend_buffer,
    decode_tagged_spend_buffer, decode_window_batch,
};
use crate::spend_queues::entity::{SpendAmount, SpendUpdateTransactions};
use crate::window_queue::WindowSpendTransaction;

fn amount(value: &str) -> SpendAmount {
    SpendAmount(value.parse::<Decimal>().unwrap())
}

fn dec(value: &str) -> Decimal {
    value.parse::<Decimal>().unwrap()
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, SpendAmount> {
    pairs
        .iter()
        .map(|(id, value)| ((*id).to_owned(), amount(value)))
        .collect()
}

#[test]
fn entity_batches_sum_every_map_and_skip_absent_or_nonmapping_fields() {
    let first = decode_entity_batch(
        br#"{"user_list_transactions":{"user-1":1.5},"end_user_list_transactions":{"end-1":0.5},"key_list_transactions":{"key-1":2},"team_list_transactions":{"team-1":1},"team_member_list_transactions":{"member-1":1},"org_list_transactions":{"org-1":1},"org_member_list_transactions":{"org-member-1":1},"project_list_transactions":{"project-1":1},"tag_list_transactions":{"tag-1":1},"agent_list_transactions":{"agent-1":1},"model_access_group_list_transactions":{"group-1":1}}"#,
    )
    .unwrap();
    let second = decode_entity_batch(
        br#"{"user_list_transactions":{"user-1":2.5,"user-2":1},"key_list_transactions":[1],"team_list_transactions":"bad"}"#,
    )
    .unwrap();
    let combined = combine_entity_transactions(&[first, second]).unwrap();
    let expected = SpendUpdateTransactions {
        user_list_transactions: map(&[("user-1", "4.0"), ("user-2", "1")]),
        end_user_list_transactions: map(&[("end-1", "0.5")]),
        key_list_transactions: map(&[("key-1", "2")]),
        team_list_transactions: map(&[("team-1", "1")]),
        team_member_list_transactions: map(&[("member-1", "1")]),
        org_list_transactions: map(&[("org-1", "1")]),
        org_member_list_transactions: map(&[("org-member-1", "1")]),
        project_list_transactions: map(&[("project-1", "1")]),
        tag_list_transactions: map(&[("tag-1", "1")]),
        agent_list_transactions: map(&[("agent-1", "1")]),
        model_access_group_spend: map(&[("group-1", "1")]),
    };
    assert_eq!(combined, expected);
}

#[test]
fn an_entity_batch_that_is_not_an_object_is_a_shape_error() {
    assert!(matches!(
        decode_entity_batch(b"[{}]"),
        Err(SpendBufferError::EntityShape)
    ));
    assert!(matches!(
        decode_entity_batch(b"null"),
        Err(SpendBufferError::EntityShape)
    ));
}

#[test]
fn an_entity_amount_that_is_not_a_number_is_rejected() {
    assert!(matches!(
        decode_entity_batch(br#"{"user_list_transactions":{"user-1":"x"}}"#),
        Err(SpendBufferError::EntityValue(_))
    ));
}

#[test]
fn combining_entity_spend_past_the_decimal_range_is_an_overflow() {
    let largest = decode_entity_batch(
        br#"{"user_list_transactions":{"user-1":79228162514264337593543950335}}"#,
    )
    .unwrap();
    assert!(matches!(
        combine_entity_transactions(&[largest.clone(), largest]),
        Err(SpendBufferError::EntityOverflow)
    ));
}

#[test]
fn a_daily_batch_decodes_its_payload_as_typed_values() {
    let daily =
        decode_daily_batch(br#"{"day-1":{"spend":1.5,"prompt_tokens":2,"note":"x"}}"#).unwrap();
    assert_eq!(
        daily["day-1"]["spend"],
        SpendColumnValue::Decimal(dec("1.5"))
    );
    assert_eq!(daily["day-1"]["prompt_tokens"], SpendColumnValue::Int(2));
    assert_eq!(
        daily["day-1"]["note"],
        SpendColumnValue::Text("x".to_owned())
    );
}

#[test]
fn a_daily_batch_that_is_not_an_object_is_a_shape_error() {
    assert!(matches!(
        decode_daily_batch(br#"[{"day-1":{}}]"#),
        Err(SpendBufferError::DailyShape)
    ));
    assert!(matches!(
        decode_daily_batch(br#"{"day-1":[1]}"#),
        Err(SpendBufferError::DailyPayload(_))
    ));
}

#[test]
fn a_window_batch_keeps_records_and_tolerates_rolled_over_request_ids() {
    let window = decode_window_batch(
        br#"[{"entity_type":"key","entity_id":"key-1","window_duration":"1h","window_start":"2024-01-01T00:00:00+00:00","spend":2.5,"request_ids":["r-1"]}]"#,
    )
    .unwrap();
    assert_eq!(
        window,
        vec![WindowSpendTransaction {
            entity_type: "key".to_owned(),
            entity_id: "key-1".to_owned(),
            window_duration: "1h".to_owned(),
            window_start: "2024-01-01T00:00:00+00:00".to_owned(),
            spend: amount("2.5"),
            started_at: None,
        }]
    );
}

#[test]
fn a_window_batch_that_is_not_an_array_or_a_record_is_rejected() {
    assert!(matches!(
        decode_window_batch(br#"{"a":1}"#),
        Err(SpendBufferError::WindowShape)
    ));
    assert!(matches!(
        decode_window_batch(br#"[{"entity_type":"key"}]"#),
        Err(SpendBufferError::WindowRecord(_))
    ));
}

#[test]
fn a_record_over_the_byte_ceiling_fails_with_the_limit() {
    let oversized = vec![b' '; MAX_SPEND_BUFFER_BYTES.saturating_add(1)];
    assert!(matches!(
        decode_spend_buffer(SpendBufferKind::Log, &oversized),
        Err(SpendBufferError::TooLarge)
    ));
    assert!(matches!(
        decode_spend_buffer(SpendBufferKind::Entity, &oversized),
        Err(SpendBufferError::TooLarge)
    ));
}

#[test]
fn a_record_of_exactly_the_byte_ceiling_is_parsed() {
    let at_limit = vec![b' '; MAX_SPEND_BUFFER_BYTES];
    assert!(matches!(
        decode_spend_buffer(SpendBufferKind::Log, &at_limit),
        Err(SpendBufferError::Log(_))
    ));
    assert!(matches!(
        decode_spend_buffer(SpendBufferKind::Daily, &at_limit),
        Err(SpendBufferError::Json(_))
    ));
}

#[test]
fn a_tagged_entry_reports_its_kind_and_the_ignored_log_row() {
    assert!(matches!(
        decode_tagged_spend_buffer(&[0, b'{', b'}']),
        Ok(Some(SpendBufferRecord::Entity(_)))
    ));
    assert!(matches!(
        decode_tagged_spend_buffer(&[1, b'{', b'}']),
        Ok(Some(SpendBufferRecord::Daily(_)))
    ));
    assert!(matches!(
        decode_tagged_spend_buffer(&[2, b'[', b']']),
        Ok(Some(SpendBufferRecord::Window(_)))
    ));
    assert!(matches!(
        decode_tagged_spend_buffer(&[3, b'[', b']']),
        Ok(Some(SpendBufferRecord::IgnoredLogRow))
    ));
    assert!(matches!(decode_tagged_spend_buffer(&[]), Ok(None)));
}

#[test]
fn a_malformed_record_propagates_its_json_error() {
    assert!(matches!(
        decode_spend_buffer(SpendBufferKind::Log, b"{"),
        Err(SpendBufferError::Log(_))
    ));
    assert!(matches!(
        decode_spend_buffer(SpendBufferKind::Daily, b"{"),
        Err(SpendBufferError::Json(_))
    ));
    assert!(matches!(
        decode_spend_buffer(SpendBufferKind::Window, b"{"),
        Err(SpendBufferError::Json(_))
    ));
}

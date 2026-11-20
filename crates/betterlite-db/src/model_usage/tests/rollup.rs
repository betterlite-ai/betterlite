use chrono::DateTime;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{key, transaction};
use crate::model_usage::rollup::{build_model_usage_transaction, model_usage_rollups};
use crate::model_usage::{
    ModelUsageError, ModelUsagePayload, ModelUsageTransaction, UsageStartTime,
};
use crate::spend_queues::entity::SpendAmount;

fn payload() -> ModelUsagePayload {
    ModelUsagePayload {
        start_time: UsageStartTime::Text(String::from("2026-01-02T10:00:00Z")),
        metadata: String::from("{}"),
        model: Some(String::from("gpt-4o")),
        model_group: Some(String::from("group")),
        custom_llm_provider: Some(String::from("openai")),
        request_tags: String::from("[]"),
        spend: SpendAmount(Decimal::new(25, 2)),
        prompt_tokens: 10,
        completion_tokens: 5,
        status: String::from("success"),
    }
}

#[test]
fn a_payload_decodes_from_its_wire_shape() {
    let decoded: ModelUsagePayload = serde_json::from_str(
        r#"{"startTime":"2026-01-02T10:00:00Z","metadata":"{}","model":"gpt-4o","request_tags":"[]","spend":0.25,"prompt_tokens":10,"completion_tokens":5,"status":"success"}"#,
    )
    .expect("a wire payload");
    assert_eq!(decoded.model, Some(String::from("gpt-4o")));
    assert_eq!(decoded.model_group, None);
    assert_eq!(decoded.custom_llm_provider, None);
    let built = build_model_usage_transaction(&decoded).expect("a dated payload");
    assert_eq!(built.key.date, "2026-01-02");
    assert_eq!(built.key.task_type, "uncategorized");
}

#[test]
fn a_payload_becomes_one_transaction() {
    let built = build_model_usage_transaction(&payload()).expect("a dated payload");
    assert_eq!(
        built,
        ModelUsageTransaction {
            key: key("2026-01-02", "group", "gpt-4o", "openai", "uncategorized"),
            spend: SpendAmount(Decimal::new(25, 2)),
            prompt_tokens: 10,
            completion_tokens: 5,
            successful: true,
        }
    );
}

#[test]
fn absent_identities_take_the_fallbacks() {
    let mut absent = payload();
    absent.model = None;
    absent.model_group = None;
    absent.custom_llm_provider = None;
    let built = build_model_usage_transaction(&absent).expect("a dated payload");
    assert_eq!(built.key.model, "unknown");
    assert_eq!(built.key.model_group, "unknown");
    assert_eq!(built.key.custom_llm_provider, "unknown");

    let mut ungrouped = payload();
    ungrouped.model_group = None;
    let grouped = build_model_usage_transaction(&ungrouped).expect("a dated payload");
    assert_eq!(grouped.key.model_group, "gpt-4o");
}

#[test]
fn explicit_identities_are_used_as_given() {
    let mut blank = payload();
    blank.model = Some(String::new());
    blank.model_group = Some(String::new());
    blank.custom_llm_provider = Some(String::new());
    let built = build_model_usage_transaction(&blank).expect("a dated payload");
    assert_eq!(built.key.model, "");
    assert_eq!(built.key.model_group, "");
    assert_eq!(built.key.custom_llm_provider, "");
}

#[rstest]
#[case::success("success", true)]
#[case::capitalized("Success", false)]
#[case::failure("failure", false)]
#[case::trailing_space("success ", false)]
#[case::empty("", false)]
fn only_the_exact_success_status_marks_a_successful_request(
    #[case] status: &str,
    #[case] successful: bool,
) {
    let mut tested = payload();
    tested.status = status.to_owned();
    let built = build_model_usage_transaction(&tested).expect("a dated payload");
    assert_eq!(built.successful, successful);
}

#[test]
fn a_truthy_internal_call_origin_suppresses_the_rollup() {
    let mut internal = payload();
    internal.metadata = String::from(r#"{"internal_call_origin": "yes"}"#);
    assert_eq!(build_model_usage_transaction(&internal), None);
    let mut falsy = payload();
    falsy.metadata = String::from(r#"{"internal_call_origin": 0}"#);
    assert!(build_model_usage_transaction(&falsy).is_some());
}

#[test]
fn a_start_time_shorter_than_ten_characters_rolls_up_nothing() {
    let mut unparsed = payload();
    unparsed.start_time = UsageStartTime::Text(String::from("2026-1-2"));
    assert_eq!(build_model_usage_transaction(&unparsed), None);
}

#[test]
fn a_datetime_start_time_uses_its_own_date() {
    let mut dated = payload();
    dated.start_time = UsageStartTime::Parsed(
        DateTime::parse_from_rfc3339("2026-01-02T23:30:00-05:00").expect("a test stamp"),
    );
    let built = build_model_usage_transaction(&dated).expect("a dated payload");
    assert_eq!(built.key.date, "2026-01-02");
}

#[test]
fn a_known_task_tag_supplies_the_task_type() {
    let mut tagged = payload();
    tagged.request_tags = String::from(r#"["task:translation","task:debugging"]"#);
    let built = build_model_usage_transaction(&tagged).expect("a dated payload");
    assert_eq!(built.key.task_type, "translation");
}

#[test]
fn rollups_group_by_the_whole_key_in_key_order() {
    let window = key("2026-01-02", "group", "gpt-4o", "openai", "uncategorized");
    let later = key("2026-01-03", "group", "gpt-4o", "openai", "uncategorized");
    let transactions = [
        transaction(window.clone(), Decimal::new(50, 2), (20, 10), false),
        transaction(later.clone(), Decimal::new(75, 2), (8, 4), true),
        transaction(window.clone(), Decimal::new(25, 2), (10, 5), true),
    ];
    let rollups = model_usage_rollups(&transactions).expect("the rollups build");
    assert_eq!(rollups.len(), 2);
    assert_eq!(rollups[0].key, window);
    assert_eq!(rollups[0].spend, Decimal::new(75, 2));
    assert_eq!(
        (rollups[0].prompt_tokens, rollups[0].completion_tokens),
        (30, 15)
    );
    assert_eq!(rollups[0].request_count, 2);
    assert_eq!(rollups[0].successful_requests, 1);
    assert_eq!(rollups[0].failed_requests, 1);
    assert_eq!(rollups[1].key, later);
    assert_eq!(rollups[1].spend, Decimal::new(75, 2));
    assert_eq!(
        (rollups[1].prompt_tokens, rollups[1].completion_tokens),
        (8, 4)
    );
    assert_eq!(rollups[1].request_count, 1);
    assert_eq!(rollups[1].successful_requests, 1);
    assert_eq!(rollups[1].failed_requests, 0);
}

#[test]
fn rollups_sort_by_date_group_model_provider_and_task() {
    let earliest = key("2026-01-01", "group", "gpt-4o", "openai", "uncategorized");
    let azure = key("2026-01-02", "group", "gpt-4o", "azure", "uncategorized");
    let model = key("2026-01-02", "group", "gpt-4o", "openai", "uncategorized");
    let debugging = key("2026-01-02", "group", "gpt-4o", "openai", "debugging");
    let tuned = key(
        "2026-01-02",
        "group",
        "gpt-4o-mini",
        "openai",
        "uncategorized",
    );
    let other_group = key("2026-01-02", "other", "gpt-4o", "openai", "uncategorized");
    let spend = Decimal::new(25, 2);
    let transactions = [
        transaction(other_group.clone(), spend, (1, 0), true),
        transaction(tuned.clone(), spend, (2, 1), true),
        transaction(debugging.clone(), spend, (4, 4), false),
        transaction(model.clone(), spend, (1, 1), true),
        transaction(azure.clone(), spend, (0, 1), true),
        transaction(earliest.clone(), spend, (3, 2), true),
    ];
    let keys: Vec<_> = model_usage_rollups(&transactions)
        .expect("the rollups build")
        .into_iter()
        .map(|entry| entry.key)
        .collect();
    assert_eq!(
        keys,
        [earliest, azure, debugging, model, tuned, other_group]
    );
}

#[test]
fn a_spend_sum_beyond_the_decimal_range_fails() {
    let largest = transaction(
        key("2026-01-02", "group", "gpt-4o", "openai", "uncategorized"),
        Decimal::MAX,
        (0, 0),
        true,
    );
    let outcome = model_usage_rollups(&[largest.clone(), largest]);
    assert!(matches!(outcome, Err(ModelUsageError::SpendOverflow)));
}

#[test]
fn a_token_sum_beyond_the_bigint_range_fails() {
    let largest = transaction(
        key("2026-01-02", "group", "gpt-4o", "openai", "uncategorized"),
        Decimal::ZERO,
        (i64::MAX, 0),
        true,
    );
    let outcome = model_usage_rollups(&[largest.clone(), largest]);
    assert!(matches!(outcome, Err(ModelUsageError::CountOverflow)));
}

#[test]
fn no_transactions_make_no_rollups() {
    assert!(
        model_usage_rollups(&[])
            .expect("nothing to group")
            .is_empty()
    );
}

#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rust_decimal::Decimal;

use super::canonical::serialize_record;
use super::decode::decode_baseline_json;
use super::{
    BaselineEntity, DailyBaselineAttribution, DailyBaselineTarget, RecordDecodeError, RecordError,
    decode_baseline_record,
};
use crate::daily_spend_sql::values::SpendColumnValue;

const DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

const VALID_RECORD: &str = r#"{"scope":"autorouter-baseline:v3:0000000000000000000000000000000000000000000000000000000000000000","api_key":"key","session_id":"session","router_name":"router","baseline_model":"baseline","observation":{"request_id":"req","started_at":1.5,"available_at":2.0,"outcome":"complete","baseline_equivalent":true},"pricing":{"model":"m","provider":"anthropic","prices":null,"basis":[null,null,null],"actual_spend":0.5,"classifier_cost":0.0},"turn":null,"daily":null}"#;

const TURN: &str = r#"{"api_key":"key","session_id":"session","router_name":"router","router_type":"simple-shuffle","model":"m","turn_at":"2026-01-02T03:04:05Z","total_tokens":15,"spend":0.5,"saved_spend":0.0,"classifier_cost":0.0,"covered":false,"cache_hit":false,"cache_ttl_seconds":300,"cache_touched":false,"tier":null,"baseline_model":"baseline","user_id":"user"}"#;

const DAILY: &str = r#"{"date":"2026-01-02","api_key":"key","model":"m","custom_llm_provider":"anthropic","model_group":null,"endpoint":"/v1/chat/completions","mcp_namespaced_tool_name":null,"targets":[{"entity":"user","entity_id":"user"},{"entity":"tag","entity_id":null}]}"#;

const COMPOSITE_PRICES: &str = r#"{"input_cost_per_token":0.5,"max_results_range":[1,2],"search_context_cost_per_query":{"search_context_size_low":0.01}}"#;

fn with_scope_digest(digest: &str) -> String {
    VALID_RECORD.replace(DIGEST, digest)
}

fn record_with(marker: &str, replacement: &str) -> String {
    VALID_RECORD.replace(marker, replacement)
}

fn with_turn(turn: &str) -> String {
    VALID_RECORD.replace("\"turn\":null", turn)
}

fn with_daily(daily: &str) -> String {
    VALID_RECORD.replace("\"daily\":null", daily)
}

fn invalid(json: &str) -> RecordError {
    match decode_baseline_record(json.as_bytes()) {
        Err(RecordDecodeError::Invalid(error)) => *error,
        Err(RecordDecodeError::Json(error)) => panic!("fixture is not valid JSON: {error}"),
        Ok(_) => panic!("fixture should have been rejected"),
    }
}

fn unreadable(json: &str) -> RecordDecodeError {
    match decode_baseline_record(json.as_bytes()) {
        Err(error @ RecordDecodeError::Json(_)) => error,
        Err(RecordDecodeError::Invalid(error)) => {
            panic!("fixture is valid JSON but invalid: {error}")
        }
        Ok(_) => panic!("fixture should have been rejected"),
    }
}

#[test]
fn canonical_serialization_matches_pythons_serialized_record_bytes() {
    let record = decode_baseline_record(include_bytes!("fixtures/canonical-record.json")).unwrap();
    let serialized = serialize_record(&record);
    let expected = include_str!("fixtures/canonical-record.serialized");
    assert_eq!(serialized, expected);
    assert_eq!(serialized.len(), 2361);
    let retried = decode_baseline_record(expected.as_bytes()).unwrap();
    assert_eq!(serialize_record(&retried), expected);
}

// The deviation in docs/deviations/B1698.yaml: a composite rate keeps its container shape but not its content.
#[test]
fn canonical_record_keeps_only_the_shape_of_composite_price_rates() {
    let json = record_with("\"prices\":null", &format!("\"prices\":{COMPOSITE_PRICES}"));
    let record = decode_baseline_record(json.as_bytes()).unwrap();
    let serialized = serialize_record(&record);
    let expected = r#""prices":{"input_cost_per_token":0.5,"max_results_range":[],"search_context_cost_per_query":{}}"#;
    assert!(serialized.contains(expected));
    let retried = decode_baseline_record(serialized.as_bytes()).unwrap();
    assert_eq!(serialize_record(&retried), serialized);
}

#[test]
fn decodes_a_stored_record_with_no_turn_or_daily_attribution() {
    let record = decode_baseline_record(VALID_RECORD.as_bytes()).unwrap();
    assert_eq!(record.api_key, "key");
    assert_eq!(record.session_id, "session");
    assert_eq!(record.router_name, "router");
    assert_eq!(record.baseline_model, "baseline");
    assert!(record.turn.is_none());
    assert!(record.daily.is_none());
    assert_eq!(record.pricing.actual_spend, Decimal::new(5, 1));
}

fn assert_invalid_scope(digest: &str) {
    let json = with_scope_digest(digest);
    assert!(matches!(invalid(&json), RecordError::InvalidScope));
}

#[test]
fn rejects_scopes_outside_the_v3_digest_shape() {
    let prefixed = record_with(
        "\"scope\":\"autorouter-baseline:v3:",
        "\"scope\":\"autorouter-baseline:v2:",
    );
    assert!(matches!(invalid(&prefixed), RecordError::InvalidScope));
    assert_invalid_scope(&"AB".repeat(32));
    assert_invalid_scope(&"a".repeat(63));
    assert_invalid_scope(&"a".repeat(65));
    assert_invalid_scope(&"g".repeat(64));
    assert_invalid_scope("");
    let valid = "ab".repeat(32);
    decode_baseline_record(with_scope_digest(&valid).as_bytes()).unwrap();
}

#[test]
fn rejects_empty_identity_fields() {
    let empty_key = record_with("\"api_key\":\"key\"", "\"api_key\":\"\"");
    assert!(matches!(invalid(&empty_key), RecordError::EmptyApiKey));
    let empty_router = record_with("\"router_name\":\"router\"", "\"router_name\":\"\"");
    assert!(matches!(
        invalid(&empty_router),
        RecordError::EmptyRouterName
    ));
    let empty_model = record_with("\"baseline_model\":\"baseline\"", "\"baseline_model\":\"\"");
    assert!(matches!(
        invalid(&empty_model),
        RecordError::EmptyBaselineModel
    ));
    let empty_session = record_with("\"session_id\":\"session\"", "\"session_id\":\"\"");
    assert!(matches!(
        invalid(&empty_session),
        RecordError::InvalidSessionId
    ));
}

#[test]
fn bounds_session_id_by_characters_not_bytes() {
    let longest = format!("\"session_id\":\"{}\"", "\u{e9}".repeat(256));
    let accepted = record_with("\"session_id\":\"session\"", &longest);
    decode_baseline_record(accepted.as_bytes()).unwrap();
    let too_long = format!("\"session_id\":\"{}\"", "\u{e9}".repeat(257));
    let rejected = record_with("\"session_id\":\"session\"", &too_long);
    assert!(matches!(invalid(&rejected), RecordError::InvalidSessionId));
}

fn assert_inconsistent_turn(turn: &str) {
    let json = with_turn(turn);
    assert!(matches!(invalid(&json), RecordError::InconsistentTurn));
}

#[test]
fn rejects_turns_that_disagree_with_the_record() {
    assert_inconsistent_turn(&TURN.replace("\"api_key\":\"key\"", "\"api_key\":\"other\""));
    assert_inconsistent_turn(
        &TURN.replace("\"session_id\":\"session\"", "\"session_id\":\"other\""),
    );
    assert_inconsistent_turn(
        &TURN.replace("\"router_name\":\"router\"", "\"router_name\":\"other\""),
    );
    assert_inconsistent_turn(&TURN.replace(
        "\"baseline_model\":\"baseline\"",
        "\"baseline_model\":\"other\"",
    ));
    assert_inconsistent_turn(
        &TURN.replace("\"baseline_model\":\"baseline\"", "\"baseline_model\":null"),
    );
    assert_inconsistent_turn(&TURN.replace("\"spend\":0.5", "\"spend\":0.6"));
    assert_inconsistent_turn(&TURN.replace("\"saved_spend\":0.0", "\"saved_spend\":0.1"));
    assert_inconsistent_turn(&TURN.replace(
        "\"user_id\":\"user\"",
        "\"user_id\":\"user\",\"savings_estimated_turns\":1",
    ));
    assert_inconsistent_turn(&TURN.replace(
        "\"user_id\":\"user\"",
        "\"user_id\":\"user\",\"savings_estimated_actual_spend\":1.0",
    ));
    assert_inconsistent_turn(&TURN.replace(
        "\"user_id\":\"user\"",
        "\"user_id\":\"user\",\"savings_estimated_saved_spend\":1.0",
    ));
    decode_baseline_record(with_turn(TURN).as_bytes()).unwrap();
}

#[test]
fn compares_turn_spend_against_actual_plus_classifier() {
    let priced = record_with(
        "\"actual_spend\":0.5,\"classifier_cost\":0.0",
        "\"actual_spend\":0.4,\"classifier_cost\":0.1",
    );
    let json = priced.replace("\"turn\":null", TURN);
    let record = decode_baseline_record(json.as_bytes()).unwrap();
    let turn = record.turn.as_ref().unwrap();
    assert_eq!(turn.spend, Decimal::new(5, 1));
    let missing_classifier = record_with(
        "\"actual_spend\":0.5,\"classifier_cost\":0.0",
        "\"actual_spend\":0.4,\"classifier_cost\":0.2",
    );
    let rejected = missing_classifier.replace("\"turn\":null", TURN);
    assert!(matches!(invalid(&rejected), RecordError::InconsistentTurn));
}

// The deviation in docs/deviations/B1698.yaml: the sum is exact, where the source adds two Python floats.
#[test]
fn compares_the_turn_spend_as_an_exact_decimal_sum() {
    let priced = record_with(
        "\"actual_spend\":0.5,\"classifier_cost\":0.0",
        "\"actual_spend\":0.1,\"classifier_cost\":0.2",
    );
    let exact = priced.replace(
        "\"turn\":null",
        &TURN.replace("\"spend\":0.5", "\"spend\":0.3"),
    );
    decode_baseline_record(exact.as_bytes()).unwrap();
    let rounded = priced.replace(
        "\"turn\":null",
        &TURN.replace("\"spend\":0.5", "\"spend\":0.30000000000000004"),
    );
    assert!(matches!(invalid(&rounded), RecordError::InconsistentTurn));
}

#[test]
fn rejects_unknown_keys_on_the_record_daily_and_target() {
    let record = record_with("\"scope\"", "\"extra\":1,\"scope\"");
    assert!(matches!(unreadable(&record), RecordDecodeError::Json(_)));
    let daily = DAILY.replace("{\"date\"", "{\"extra\":1,\"date\"");
    let attributed = with_daily(&daily);
    assert!(matches!(
        unreadable(&attributed),
        RecordDecodeError::Json(_)
    ));
    let target = DAILY.replace("{\"entity\":\"tag\"", "{\"entity\":\"tag\",\"extra\":1");
    let with_extra_target = with_daily(&target);
    assert!(matches!(
        unreadable(&with_extra_target),
        RecordDecodeError::Json(_)
    ));
}

#[test]
fn decodes_daily_attribution_targets_in_order() {
    let json = with_daily(DAILY);
    let record = decode_baseline_record(json.as_bytes()).unwrap();
    let daily = record.daily.unwrap();
    assert_eq!(daily.date, "2026-01-02");
    assert_eq!(daily.api_key, "key");
    assert_eq!(daily.model.as_deref(), Some("m"));
    assert_eq!(daily.custom_llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(daily.model_group, None);
    assert_eq!(daily.endpoint.as_deref(), Some("/v1/chat/completions"));
    assert_eq!(daily.mcp_namespaced_tool_name, None);
    assert_eq!(
        daily.targets,
        vec![
            DailyBaselineTarget {
                entity: BaselineEntity::User,
                entity_id: Some("user".to_owned()),
            },
            DailyBaselineTarget {
                entity: BaselineEntity::Tag,
                entity_id: None,
            },
        ]
    );
}

#[test]
fn defaults_missing_daily_targets_to_an_empty_tuple() {
    let minimal = r#"{"date":"2026-01-02","api_key":"key"}"#;
    let json = with_daily(minimal);
    let record = decode_baseline_record(json.as_bytes()).unwrap();
    assert!(record.daily.unwrap().targets.is_empty());
}

fn attribution(model: Option<&str>) -> DailyBaselineAttribution {
    DailyBaselineAttribution {
        date: "2026-01-02".to_owned(),
        api_key: "key".to_owned(),
        model: model.map(str::to_owned),
        custom_llm_provider: None,
        model_group: None,
        endpoint: None,
        mcp_namespaced_tool_name: None,
        targets: Vec::new(),
    }
}

fn assert_adjustment_targets(entity: BaselineEntity, column: &str) {
    let target = DailyBaselineTarget {
        entity,
        entity_id: Some(format!("{column}-value")),
    };
    let row = attribution(Some("m")).adjustment(&target, Decimal::new(-25, 2), "req");
    assert_eq!(
        row.get(column),
        Some(&SpendColumnValue::Text(format!("{column}-value")))
    );
    assert_eq!(row.len(), 10);
    assert_eq!(
        row.get("date"),
        Some(&SpendColumnValue::Text("2026-01-02".to_owned()))
    );
    assert_eq!(
        row.get("request_id"),
        Some(&SpendColumnValue::Text("req".to_owned()))
    );
    assert_eq!(
        row.get("autorouter_savings_spend"),
        Some(&SpendColumnValue::Decimal(Decimal::new(-25, 2)))
    );
    assert!(!row.contains_key("spend"));
    assert!(!row.contains_key("total_tokens"));
    assert!(!row.contains_key("prompt_tokens"));
    assert!(!row.contains_key("completion_tokens"));
}

#[test]
fn adjustment_uses_each_entity_id_column() {
    assert_adjustment_targets(BaselineEntity::User, "user_id");
    assert_adjustment_targets(BaselineEntity::Team, "team_id");
    assert_adjustment_targets(BaselineEntity::Org, "organization_id");
    assert_adjustment_targets(BaselineEntity::Tag, "tag");
    assert_adjustment_targets(BaselineEntity::EndUser, "end_user_id");
    assert_adjustment_targets(BaselineEntity::Agent, "agent_id");
}

#[test]
fn adjustment_keeps_a_null_entity_id_for_an_unscoped_target() {
    let target = DailyBaselineTarget {
        entity: BaselineEntity::User,
        entity_id: None,
    };
    let row = attribution(None).adjustment(&target, Decimal::ZERO, "req");
    assert_eq!(row.get("user_id"), Some(&SpendColumnValue::Null));
    assert_eq!(row.get("model"), Some(&SpendColumnValue::Null));
}

#[rstest::rstest]
fn replays_the_baseline_json_corpus(#[files("fuzz/corpus/baseline_json/*")] path: PathBuf) {
    let data = fs::read(path).unwrap();
    drop(decode_baseline_json(&data));
}

#![cfg(test)]

use rstest::rstest;
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::time::Duration;

use crate::key_hash::KeyHash;
use crate::key_repository::table::TokenValue;
use crate::migration_ledger::lock::JitterSource;
use crate::postgres_store::errors::DbError;
use crate::table_ops::columns::TeamColumn;
use crate::table_ops::rows::SpendLogRow;

use super::sql::{self, InsertConflict};
use super::{
    DEFAULT_TEAM_LIST_LIMIT, OperationCallType, TableOps, TableOpsError, TablePorts, TableSettings,
};
use support::{Ports, TableClient};

mod failures;
mod generic_filters;
mod insert_team;
mod reads;
pub(super) mod support;
mod timeouts;
mod tls;
mod writes;

#[test]
fn each_operation_keeps_litellms_call_type_name() {
    assert_eq!(OperationCallType::GetData.as_str(), "get_data");
    assert_eq!(
        OperationCallType::GetGenericData.as_str(),
        "get_generic_data"
    );
    assert_eq!(OperationCallType::InsertData.as_str(), "insert_data");
    assert_eq!(OperationCallType::UpdateData.as_str(), "update_data");
    assert_eq!(OperationCallType::DeleteData.as_str(), "delete_data");
}

#[test]
fn the_team_list_defaults_to_twenty() {
    assert_eq!(
        TableSettings::default().team_list_limit,
        DEFAULT_TEAM_LIST_LIMIT
    );
    assert_eq!(DEFAULT_TEAM_LIST_LIMIT, 20);
}

#[test]
fn a_missing_database_is_not_reported_as_a_database_fault() {
    let error = TableOpsError::<Infallible>::NoDbConnected;
    assert!(error.database_source().is_none());
}

#[test]
fn a_database_error_keeps_its_source() {
    let error = TableOpsError::<Infallible>::Database {
        source: DbError::NoDbConnected,
    };
    assert!(matches!(
        error.database_source(),
        Some(DbError::NoDbConnected)
    ));
}

struct FullJitter;

impl JitterSource for FullJitter {
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64 {
        upper_exclusive_nanos
    }
}

#[test]
fn backoff_doubles_each_try_and_stops_at_the_budget() {
    let ports = Ports::new();
    let jitter = FullJitter;
    let operations: TableOps<'_, TableClient> = TableOps {
        database: None,
        ports: TablePorts {
            clock: &ports.clock,
            jitter: &jitter,
            pause: &ports.pause,
            sink: &ports.sink,
            invalidator: &ports.invalidator,
        },
        settings: TableSettings::default(),
    };
    let budget = Duration::from_secs(10);
    assert_eq!(
        operations.backoff(1, Duration::ZERO, budget),
        Duration::from_secs(1)
    );
    assert_eq!(
        operations.backoff(2, Duration::ZERO, budget),
        Duration::from_secs(2)
    );
    assert_eq!(
        operations.backoff(3, Duration::ZERO, budget),
        Duration::from_secs(4)
    );
    assert_eq!(operations.backoff(5, Duration::ZERO, budget), budget);
    assert_eq!(
        operations.backoff(2, Duration::from_secs(9), budget),
        Duration::from_secs(1)
    );
}

#[test]
fn the_spend_log_debug_prints_no_api_key() {
    let row: SpendLogRow = serde_json::from_str(
        r#"{"request_id":"req-1","call_type":"acompletion","api_key":"sk-secret","spend":1.5,"startTime":"2024-01-01T00:00:00","endTime":"2024-01-01T00:00:00","created_at":"2024-01-01T00:00:00","updated_at":"2024-01-01T00:00:00"}"#,
    )
    .expect("the spend row parses");
    let printed = format!("{row:?}");
    assert!(printed.contains("SpendLogRow"));
    assert!(printed.contains("req-1"));
    assert!(!printed.contains("sk-secret"));
}

#[test]
fn a_hashed_key_binds_into_a_text_column() {
    let payload = BTreeMap::from([
        (TeamColumn::TeamId, TokenValue::Text(String::from("team-1"))),
        (
            TeamColumn::TeamAlias,
            TokenValue::KeyHash(KeyHash::from("hashed-key")),
        ),
    ]);
    let statement = sql::insert::<Infallible, TeamColumn>(&payload, InsertConflict::Keep)
        .expect("the hashed key binds as text");
    assert!(statement.sql.contains("\"team_alias\""));
    assert_eq!(statement.parameters.values.len(), 2);
}

#[test]
fn integer_values_bind_as_bigint_for_every_integer_column() {
    let payload = BTreeMap::from([
        (TeamColumn::TeamId, TokenValue::Text(String::from("team-1"))),
        (TeamColumn::MaxParallelRequests, TokenValue::Integer(5)),
        (TeamColumn::TpmLimit, TokenValue::Null),
    ]);
    let statement = sql::insert::<Infallible, TeamColumn>(&payload, InsertConflict::Keep)
        .expect("the integers bind with a width cast");
    assert!(statement.sql.contains("$2::bigint"));
    assert!(statement.sql.contains("$3::bigint"));
}

#[test]
fn null_and_bool_values_bind_their_column_types() {
    let with_nulls = BTreeMap::from([
        (TeamColumn::TeamId, TokenValue::Text(String::from("team-1"))),
        (TeamColumn::TeamAlias, TokenValue::Null),
        (TeamColumn::Blocked, TokenValue::Null),
        (TeamColumn::MaxParallelRequests, TokenValue::Null),
        (TeamColumn::Models, TokenValue::Null),
    ]);
    let nulls = sql::insert::<Infallible, TeamColumn>(&with_nulls, InsertConflict::Keep)
        .expect("the nulls bind their column types");
    assert_eq!(nulls.parameters.values.len(), 5);
    let with_flag = BTreeMap::from([
        (TeamColumn::TeamId, TokenValue::Text(String::from("team-1"))),
        (TeamColumn::Blocked, TokenValue::Bool(true)),
    ]);
    let flag = sql::insert::<Infallible, TeamColumn>(&with_flag, InsertConflict::Keep)
        .expect("the flag binds as a boolean");
    assert_eq!(flag.parameters.values.len(), 2);
}

#[rstest]
#[case::text_to_bool(TeamColumn::Blocked, TokenValue::Text(String::from("true")))]
#[case::hash_to_bool(TeamColumn::Blocked, TokenValue::KeyHash(KeyHash::from("hash")))]
#[case::bool_to_text(TeamColumn::TeamAlias, TokenValue::Bool(true))]
#[case::integer_to_text(TeamColumn::TeamAlias, TokenValue::Integer(5))]
#[case::array_to_text(TeamColumn::TeamAlias, TokenValue::TextArray(vec![String::from("value")]))]
fn scalar_values_reject_the_wrong_column_type(
    #[case] column: TeamColumn,
    #[case] value: TokenValue,
) {
    let payload = BTreeMap::from([
        (TeamColumn::TeamId, TokenValue::Text(String::from("team-1"))),
        (column, value),
    ]);
    assert!(matches!(
        sql::insert::<Infallible, TeamColumn>(&payload, InsertConflict::Keep),
        Err(TableOpsError::WrongType { .. })
    ));
}

#[rstest]
#[case::object(r#", "model_max_budget":{"gpt":1}"#, true)]
#[case::text(r#", "model_max_budget":"{\"gpt\":1}""#, true)]
#[case::null(r#", "model_max_budget":null"#, false)]
#[case::absent("", false)]
fn budget_maps_decode_from_objects_text_and_null(#[case] field: &str, #[case] populated: bool) {
    use crate::budget_permission_rows::LiteLLMBudgetTableFull;
    use crate::table_ops::rows::RowDecoder as _;

    let text = format!(r#"{{"budget_id":"b-1","created_at":"2024-01-01T00:00:00"{field}}}"#);
    let row = LiteLLMBudgetTableFull::decode_row(&text).unwrap();
    assert_eq!(row.budget.budget_id.as_deref(), Some("b-1"));
    assert_eq!(row.budget.model_max_budget.is_some(), populated);
    if let Some(fields) = row.budget.model_max_budget {
        assert_eq!(serde_json::to_string(&fields).unwrap(), r#"{"gpt":1}"#);
    }
}

#[test]
fn invalid_budget_map_text_is_rejected() {
    use crate::budget_permission_rows::LiteLLMBudgetTableFull;
    use crate::table_ops::rows::RowDecoder as _;
    assert!(
        LiteLLMBudgetTableFull::decode_row(
            r#"{"created_at":"2024-01-01T00:00:00","model_max_budget":"invalid"}"#
        )
        .is_err()
    );
}

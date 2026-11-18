#![cfg(test)]

use std::convert::Infallible;

use super::{DEFAULT_TEAM_LIST_LIMIT, OperationCallType, TableOpsError, TableSettings};

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

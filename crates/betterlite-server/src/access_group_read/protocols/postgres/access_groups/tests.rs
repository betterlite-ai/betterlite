#![cfg(test)]

use crate::access_group_read::protocols::records::{AccessGroupChanges, FieldWrite};

use super::update_statement;

fn untouched() -> AccessGroupChanges {
    AccessGroupChanges {
        access_group_name: None,
        description: FieldWrite::Unchanged,
        access_model_names: None,
        access_mcp_server_ids: None,
        access_agent_ids: None,
        assigned_team_ids: None,
        assigned_key_ids: None,
        updated_by: None,
    }
}

#[test]
fn update_statement_always_moves_updated_at_and_writes_updated_by() {
    let (statement, parameters) = update_statement("group-1", &untouched());
    assert_eq!(parameters.len(), 2_usize);
    assert!(statement.contains(r#""updated_by" = $2"#));
    assert!(statement.contains(r#""updated_at" = (NOW() AT TIME ZONE 'UTC')"#));
    assert!(statement.contains(r#"WHERE "access_group_id" = $1"#));
    let set_clause = statement.split(" WHERE ").next().expect("a set clause");
    assert!(!set_clause.contains("access_group_name"));
    assert!(!set_clause.contains("description"));
    assert!(!set_clause.contains("assigned_key_ids"));
}

#[test]
fn update_statement_numbers_only_the_columns_the_caller_set() {
    let changes = AccessGroupChanges {
        access_group_name: Some("renamed".to_owned()),
        description: FieldWrite::Null,
        access_model_names: Some(vec!["model-b".to_owned()]),
        access_mcp_server_ids: None,
        access_agent_ids: None,
        assigned_team_ids: None,
        assigned_key_ids: None,
        updated_by: Some("editor".to_owned()),
    };
    let (statement, parameters) = update_statement("group-1", &changes);
    assert_eq!(parameters.len(), 4_usize);
    assert!(statement.contains(r#""access_group_name" = $3"#));
    assert!(statement.contains(r#""description" = NULL"#));
    assert!(statement.contains(r#""access_model_names" = $4"#));
    assert!(!statement.contains(r#""access_mcp_server_ids" = $5"#));
}

use crate::mcp::table_access::filter::{FindServers, ServerFilter};
use crate::mcp::table_access::rows::{CREDENTIAL_TABLE, KeyValues, RowFields, SERVER_TABLE};
use crate::mcp::table_access::sql::{
    Select, delete_statement, insert_sql, key_predicate, params, select_sql, update_statement,
};
use crate::mcp::table_access::types::IdentifierField;

fn fields() -> RowFields {
    RowFields::texts([("server_id", "s1"), ("transport", "sse")])
}

#[test]
fn insert_quotes_every_column_and_returns_the_row() {
    let (query, binds) = insert_sql(&SERVER_TABLE, &fields(), false).expect("the row serializes");
    assert_eq!(
        query,
        "INSERT INTO \"LiteLLM_MCPServerTable\" AS t (\"server_id\", \"transport\") SELECT \"server_id\", \"transport\" FROM jsonb_populate_record(NULL::\"LiteLLM_MCPServerTable\", $1::text::jsonb) RETURNING row_to_json(t)::text"
    );
    assert_eq!(binds, vec![r#"{"server_id":"s1","transport":"sse"}"#]);
}

#[test]
fn insert_on_conflict_updates_every_mutable_column_and_the_timestamp() {
    let (query, binds) = insert_sql(&SERVER_TABLE, &fields(), true).expect("the row serializes");
    assert_eq!(
        query,
        "INSERT INTO \"LiteLLM_MCPServerTable\" AS t (\"server_id\", \"transport\") SELECT \"server_id\", \"transport\" FROM jsonb_populate_record(NULL::\"LiteLLM_MCPServerTable\", $1::text::jsonb) ON CONFLICT (\"server_id\") DO UPDATE SET \"transport\" = EXCLUDED.\"transport\", \"updated_at\" = now() RETURNING row_to_json(t)::text"
    );
    assert_eq!(binds.len(), 1);
}

#[test]
fn update_numbers_binds_after_the_json_and_honours_the_returning_flag() {
    let data = RowFields::texts([("approval_status", "active")]);
    let filter = KeyValues::from([("server_id", "s1".to_owned())]);
    let (query, binds) =
        update_statement(&SERVER_TABLE, &data, &filter, true).expect("the row serializes");
    assert_eq!(
        query,
        "UPDATE \"LiteLLM_MCPServerTable\" AS t SET (\"approval_status\") = (SELECT \"approval_status\" FROM jsonb_populate_record(NULL::\"LiteLLM_MCPServerTable\", $1::text::jsonb)), \"updated_at\" = now() WHERE \"server_id\" = $2 RETURNING row_to_json(t)::text"
    );
    assert_eq!(
        binds,
        vec![
            r#"{"approval_status":"active"}"#.to_owned(),
            "s1".to_owned()
        ]
    );
    let (quiet, quiet_binds) = update_statement(&SERVER_TABLE, &data, &KeyValues::new(), false)
        .expect("the row serializes");
    assert_eq!(
        quiet,
        "UPDATE \"LiteLLM_MCPServerTable\" AS t SET (\"approval_status\") = (SELECT \"approval_status\" FROM jsonb_populate_record(NULL::\"LiteLLM_MCPServerTable\", $1::text::jsonb)), \"updated_at\" = now() WHERE TRUE"
    );
    assert_eq!(quiet_binds, vec![r#"{"approval_status":"active"}"#]);
}

#[test]
fn delete_predicates_without_a_filter_and_keeps_the_returning_flag() {
    let filter = KeyValues::from([("server_id", "s1".to_owned())]);
    let (query, binds) = delete_statement(&SERVER_TABLE, &filter, true);
    assert_eq!(
        query,
        "DELETE FROM \"LiteLLM_MCPServerTable\" AS t WHERE \"server_id\" = $1 RETURNING row_to_json(t)::text"
    );
    assert_eq!(binds, vec!["s1".to_owned()]);
    let (quiet, quiet_binds) = delete_statement(&CREDENTIAL_TABLE, &KeyValues::new(), false);
    assert_eq!(
        quiet,
        "DELETE FROM \"LiteLLM_MCPUserCredentials\" AS t WHERE TRUE"
    );
    assert!(quiet_binds.is_empty());
}

#[test]
fn key_predicates_are_anded_in_key_order() {
    let filter = KeyValues::from([("user_id", "u1".to_owned()), ("server_id", "s1".to_owned())]);
    assert_eq!(
        key_predicate(&filter, 3),
        Some((
            "\"server_id\" = $3 AND \"user_id\" = $4".to_owned(),
            vec!["s1".to_owned(), "u1".to_owned()]
        ))
    );
    assert_eq!(key_predicate(&KeyValues::new(), 1), None);
}

#[test]
fn server_predicates_render_each_filter_arm() {
    assert_eq!(
        ServerFilter::ServerId("s1".to_owned()).predicate(1),
        Some(("\"server_id\" = $1".to_owned(), vec!["s1".to_owned()]))
    );
    assert_eq!(
        ServerFilter::ServerIds(Vec::new()).predicate(1),
        Some(("FALSE".to_owned(), Vec::new()))
    );
    assert_eq!(
        ServerFilter::ServerIds(vec!["a".to_owned(), "b".to_owned()]).predicate(2),
        Some((
            "\"server_id\" IN ($2, $3)".to_owned(),
            vec!["a".to_owned(), "b".to_owned()]
        ))
    );
    assert_eq!(
        ServerFilter::ApprovalStatus("draft".to_owned()).predicate(1),
        Some((
            "\"approval_status\" = $1".to_owned(),
            vec!["draft".to_owned()]
        ))
    );
    assert_eq!(
        ServerFilter::NotDraft.predicate(1),
        Some((
            "(\"approval_status\" IS NULL OR \"approval_status\" <> 'draft')".to_owned(),
            Vec::new()
        ))
    );
    assert_eq!(
        ServerFilter::SubmittedByActive {
            user_id: "u1".to_owned()
        }
        .predicate(1),
        Some((
            "\"submitted_by\" = $1 AND \"approval_status\" = 'active'".to_owned(),
            vec!["u1".to_owned()]
        ))
    );
    assert_eq!(
        ServerFilter::Submitted.predicate(1),
        Some(("\"submitted_at\" IS NOT NULL".to_owned(), Vec::new()))
    );
    let unfiltered = FindServers {
        filter: None,
        descending: false,
        take: None,
    };
    assert_eq!(unfiltered.predicate(1), None);
    let filtered = FindServers {
        filter: Some(ServerFilter::Submitted),
        descending: false,
        take: None,
    };
    assert_eq!(
        filtered.predicate(1),
        Some(("\"submitted_at\" IS NOT NULL".to_owned(), Vec::new()))
    );
}

#[test]
fn select_clauses_follow_the_predicate_order_and_limit() {
    let binds = ["s1".to_owned()];
    let wide = Select {
        predicate: Some("\"server_id\" = $1".to_owned()),
        binds: &binds,
        descending: true,
        take: Some(2),
    };
    assert_eq!(
        select_sql(&SERVER_TABLE, &wide),
        "SELECT row_to_json(t)::text FROM \"LiteLLM_MCPServerTable\" t WHERE \"server_id\" = $1 ORDER BY t.\"submitted_at\" DESC LIMIT 2"
    );
    let plain = Select {
        predicate: None,
        binds: &[],
        descending: false,
        take: None,
    };
    assert_eq!(
        select_sql(&CREDENTIAL_TABLE, &plain),
        "SELECT row_to_json(t)::text FROM \"LiteLLM_MCPUserCredentials\" t"
    );
}

#[test]
fn params_binds_every_value_in_order() {
    assert!(params(&[]).is_empty());
    assert_eq!(params(&["a".to_owned(), "b".to_owned()]).len(), 2);
}

#[test]
fn identifier_fields_render_their_column_names() {
    assert_eq!(IdentifierField::Alias.as_str(), "alias");
    assert_eq!(IdentifierField::ServerName.as_str(), "server_name");
}

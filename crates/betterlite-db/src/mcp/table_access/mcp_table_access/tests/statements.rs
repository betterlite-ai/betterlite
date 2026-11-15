use crate::mcp::table_access::filter::{FindServers, ServerFilter};
use crate::mcp::table_access::rows::{CREDENTIAL_TABLE, KeyValues, RowFields, SERVER_TABLE};
use crate::mcp::table_access::sql::{
    Scope, Select, delete_statement, insert_sql, key_predicate, params, select_sql,
    update_statement,
};
use crate::mcp::table_access::types::IdentifierField;

fn fields() -> RowFields {
    RowFields::texts(&[("server_id", "s1"), ("transport", "sse")])
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
fn a_credential_upsert_conflicts_on_the_compound_key_and_rewrites_only_the_blob() {
    let data = RowFields::texts(&[
        ("credential_b64", "blob"),
        ("id", "fresh"),
        ("server_id", "s1"),
        ("user_id", "u1"),
    ]);
    let (query, _binds) = insert_sql(&CREDENTIAL_TABLE, &data, true).expect("the row serializes");
    assert!(query.contains(
        " ON CONFLICT (\"user_id\", \"server_id\") DO UPDATE SET \"credential_b64\" = EXCLUDED.\"credential_b64\", \"updated_at\" = now() RETURNING"
    ));
}

#[test]
fn update_numbers_binds_after_the_json() {
    let data = RowFields::texts(&[("approval_status", "active")]);
    let filter = KeyValues::from([("server_id", "s1".to_owned())]);
    let (query, binds) =
        update_statement(&SERVER_TABLE, &data, &filter, Scope::Unique).expect("the row serializes");
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
}

#[test]
fn an_unfiltered_update_touches_no_row_when_unique_and_every_row_when_many() {
    let data = RowFields::texts(&[("approval_status", "active")]);
    let (unique, unique_binds) =
        update_statement(&SERVER_TABLE, &data, &KeyValues::new(), Scope::Unique)
            .expect("the row serializes");
    assert!(unique.ends_with(" WHERE FALSE RETURNING row_to_json(t)::text"));
    assert_eq!(unique_binds, vec![r#"{"approval_status":"active"}"#]);
    let (many, _many_binds) =
        update_statement(&SERVER_TABLE, &data, &KeyValues::new(), Scope::Many)
            .expect("the row serializes");
    assert!(many.ends_with(" WHERE TRUE RETURNING row_to_json(t)::text"));
}

#[test]
fn delete_keys_its_predicate_and_falls_back_by_scope() {
    let filter = KeyValues::from([("server_id", "s1".to_owned())]);
    let (query, binds) = delete_statement(&SERVER_TABLE, &filter, Scope::Unique);
    assert_eq!(
        query,
        "DELETE FROM \"LiteLLM_MCPServerTable\" AS t WHERE \"server_id\" = $1 RETURNING row_to_json(t)::text"
    );
    assert_eq!(binds, vec!["s1".to_owned()]);
    let (unique, unique_binds) =
        delete_statement(&CREDENTIAL_TABLE, &KeyValues::new(), Scope::Unique);
    assert_eq!(
        unique,
        "DELETE FROM \"LiteLLM_MCPUserCredentials\" AS t WHERE FALSE RETURNING row_to_json(t)::text"
    );
    assert!(unique_binds.is_empty());
    let (many, _many_binds) = delete_statement(&CREDENTIAL_TABLE, &KeyValues::new(), Scope::Many);
    assert_eq!(
        many,
        "DELETE FROM \"LiteLLM_MCPUserCredentials\" AS t WHERE TRUE RETURNING row_to_json(t)::text"
    );
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
        ("\"server_id\" = $1".to_owned(), vec!["s1".to_owned()])
    );
    assert_eq!(
        ServerFilter::ServerIds(Vec::new()).predicate(1),
        ("FALSE".to_owned(), Vec::new())
    );
    assert_eq!(
        ServerFilter::ServerIds(vec!["a".to_owned(), "b".to_owned()]).predicate(2),
        (
            "\"server_id\" IN ($2, $3)".to_owned(),
            vec!["a".to_owned(), "b".to_owned()]
        )
    );
    assert_eq!(
        ServerFilter::ApprovalStatus("draft".to_owned()).predicate(1),
        (
            "\"approval_status\" = $1".to_owned(),
            vec!["draft".to_owned()]
        )
    );
    assert_eq!(
        ServerFilter::NotDraft.predicate(1),
        (
            "(\"approval_status\" IS NULL OR \"approval_status\" <> 'draft')".to_owned(),
            Vec::new()
        )
    );
    assert_eq!(
        ServerFilter::SubmittedByActive {
            user_id: "u1".to_owned()
        }
        .predicate(1),
        (
            "\"submitted_by\" = $1 AND \"approval_status\" = 'active'".to_owned(),
            vec!["u1".to_owned()]
        )
    );
    assert_eq!(
        ServerFilter::Submitted.predicate(1),
        ("\"submitted_at\" IS NOT NULL".to_owned(), Vec::new())
    );
}

#[test]
fn an_identifier_predicate_without_an_excluded_id_binds_only_the_value() {
    assert_eq!(
        ServerFilter::Identifiers {
            value: "Shared".to_owned(),
            exclude_server_id: None,
        }
        .predicate(4),
        (
            "((LOWER(\"server_name\") = LOWER($4) OR LOWER(\"alias\") = LOWER($4)) AND (\"approval_status\" IS NULL OR \"approval_status\" <> 'draft'))".to_owned(),
            vec!["Shared".to_owned()]
        )
    );
}

#[test]
fn a_find_request_renders_its_filter_order_and_limit() {
    let request = FindServers {
        filter: Some(ServerFilter::ServerId("s1".to_owned())),
        descending: true,
        take: Some(2),
    };
    assert_eq!(
        select_sql(&SERVER_TABLE, &Select::from(&request)),
        (
            "SELECT row_to_json(t)::text FROM \"LiteLLM_MCPServerTable\" t WHERE \"server_id\" = $1 ORDER BY t.\"submitted_at\" DESC LIMIT 2".to_owned(),
            vec!["s1".to_owned()]
        )
    );
    let unfiltered = FindServers {
        filter: None,
        descending: false,
        take: None,
    };
    assert_eq!(
        select_sql(&CREDENTIAL_TABLE, &Select::from(&unfiltered)),
        (
            "SELECT row_to_json(t)::text FROM \"LiteLLM_MCPUserCredentials\" t".to_owned(),
            Vec::new()
        )
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

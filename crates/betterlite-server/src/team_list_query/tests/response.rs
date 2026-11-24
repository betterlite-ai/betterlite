use std::collections::BTreeMap;

use betterlite_db::team_rows::team::LiteLLMTeamTable;

use crate::team_list_query::response::convert_teams_to_response_models;
use crate::team_request_contracts::team_list::TeamListEntry;

use super::support::{data, record};

fn enriched(entry: &TeamListEntry) -> (&str, i64, i64) {
    let TeamListEntry::Enriched(item) = entry else {
        panic!("a live row converts to the enriched model");
    };
    (
        item.team.fields.base.team_id.as_str(),
        item.members_count,
        item.keys_count,
    )
}

// model_dump and its dict fallback both reach the same conversion here
#[test]
fn model_dump_fallback() {
    let text = r#"{"team_id":"t1","team_alias":"Alias","members_with_roles":[{"user_id":"u1","role":"user"}]}"#;
    let typed = LiteLLMTeamTable::try_from(data(text)).unwrap();
    let dumped = record(&serde_json::to_string(&typed).unwrap());
    let converted = convert_teams_to_response_models(&[dumped], false, None).unwrap();
    let direct = convert_teams_to_response_models(&[record(text)], false, None).unwrap();
    assert_eq!(converted, direct);
    assert_eq!(enriched(&converted[0]), ("t1", 1, 0));
}

#[test]
fn nonlist_roster_empty() {
    let rows = [
        record(r#"{"team_id":"t1","members_with_roles":"not-a-list"}"#),
        record(r#"{"team_id":"t2","members_with_roles":null}"#),
    ];
    let converted = convert_teams_to_response_models(&rows, false, None).unwrap();
    let counts: Vec<(i64, usize)> = converted
        .iter()
        .map(|entry| {
            let TeamListEntry::Enriched(item) = entry else {
                panic!("a live row converts to the enriched model");
            };
            (
                item.members_count,
                item.team.fields.base.members_with_roles.len(),
            )
        })
        .collect();
    assert_eq!(counts, vec![(0_i64, 0_usize), (0, 0)]);
}

#[test]
fn missing_count_zero() {
    let mut counts = BTreeMap::new();
    counts.insert("other".to_owned(), 4_i64);
    counts.insert(String::new(), 7_i64);
    let converted =
        convert_teams_to_response_models(&[record(r#"{"team_id":"t1"}"#)], false, Some(&counts))
            .unwrap();
    assert_eq!(enriched(&converted[0]), ("t1", 0, 0));
    // team_dict.get("team_id") or "": an empty ID reads the "" key
    let empty_id =
        convert_teams_to_response_models(&[record(r#"{"team_id":""}"#)], false, Some(&counts))
            .unwrap();
    assert_eq!(enriched(&empty_id[0]), ("", 0, 7));
}

#[test]
fn live_counts() {
    let text = r#"{"team_id":"t1","members_with_roles":[{"user_id":"u1","role":"admin"},{"user_id":"u2","role":"user"}]}"#;
    let mut counts = BTreeMap::new();
    counts.insert("t1".to_owned(), 5_i64);
    let converted =
        convert_teams_to_response_models(&[record(text)], false, Some(&counts)).unwrap();
    assert_eq!(enriched(&converted[0]), ("t1", 2, 5));
    let TeamListEntry::Enriched(item) = &converted[0] else {
        panic!("a live row converts to the enriched model");
    };
    assert_eq!(item.access_group_models, None);
    assert_eq!(item.access_group_mcp_server_ids, None);
    assert_eq!(item.access_group_agent_ids, None);
}

#[test]
fn deleted_row_projection() {
    let text = r#"{"team_id":"t1","id":"deleted-1","deleted_by":"admin","deleted_by_api_key":"key-hash","litellm_changed_by":"ops"}"#;
    let converted = convert_teams_to_response_models(&[record(text)], true, None).unwrap();
    let TeamListEntry::Deleted(row) = &converted[0] else {
        panic!("a deleted row keeps the deleted-team model");
    };
    assert_eq!(row.team.fields.base.team_id, "t1");
    assert_eq!(row.audit.id.as_deref(), Some("deleted-1"));
    assert_eq!(row.audit.deleted_by.as_deref(), Some("admin"));
    assert_eq!(row.audit.deleted_by_api_key.as_deref(), Some("key-hash"));
    assert_eq!(row.audit.litellm_changed_by.as_deref(), Some("ops"));
    assert_eq!(row.audit.deleted_at, None);
}

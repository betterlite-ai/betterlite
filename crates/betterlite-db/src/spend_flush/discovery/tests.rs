#![cfg(test)]

use super::record_items;
use crate::tool_discovery::ToolDiscoveryItem;
use crate::tool_policy_store::records::ToolDiscoveryItem as ToolRecord;

// One distinct value per field, so a record built from the wrong field shows.
fn discovered() -> ToolDiscoveryItem {
    ToolDiscoveryItem {
        tool_name: Some("search".to_owned()),
        origin: Some("mcp".to_owned()),
        created_by: Some("user-a".to_owned()),
        key_hash: Some("hash-a".to_owned()),
        team_id: Some("team-a".to_owned()),
        key_alias: Some("alias-a".to_owned()),
        user_agent: Some("client/1.0".to_owned()),
    }
}

fn expected_record() -> ToolRecord {
    ToolRecord {
        tool_name: Some("search".to_owned()),
        origin: Some("mcp".to_owned()),
        created_by: Some("user-a".to_owned()),
        key_hash: Some("hash-a".to_owned()),
        team_id: Some("team-a".to_owned()),
        key_alias: Some("alias-a".to_owned()),
        user_agent: Some("client/1.0".to_owned()),
    }
}

#[test]
fn every_drained_tool_becomes_one_record_with_all_its_fields() {
    let records = record_items(&[discovered(), discovered()]);

    assert_eq!(records, [expected_record(), expected_record()]);
}

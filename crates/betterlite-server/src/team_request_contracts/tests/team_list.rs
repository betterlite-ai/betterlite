use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::archived::LiteLLMDeletedTeamTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use super::support::{object, parse};
use crate::team_request_contracts::shared::to_data;
use crate::team_request_contracts::team_list::{TeamListEntry, TeamListItem, TeamListResponse};

const ENRICHED_ROW: &str = r#"{
    "team_id": "team-live", "team_alias": "live", "members_count": 3, "keys_count": 2,
    "access_group_models": ["gpt-4o"], "access_group_mcp_server_ids": null,
    "access_group_agent_ids": ["agent-1"]
}"#;

const LIVE_ROW: &str = r#"{"team_id": "team-bare"}"#;

const DELETED_ROW: &str = r#"{"team_id": "team-gone", "team_alias": "gone", "deleted_by": "admin", "deleted_at": "2024-06-01T00:00:00Z"}"#;

fn union_document() -> JsonData {
    parse(&format!(
        r#"{{"teams": [{ENRICHED_ROW}, {LIVE_ROW}, {DELETED_ROW}], "total": 3, "page": 1, "page_size": 10, "total_pages": 1}}"#
    ))
}

#[test]
fn list_live_deleted_enriched_union() {
    let enriched = TeamListItem::try_from(&parse(ENRICHED_ROW)).unwrap();
    assert_eq!(enriched.team.fields.base.team_id, "team-live");
    assert_eq!(enriched.members_count, 3);
    assert_eq!(enriched.keys_count, 2);
    assert_eq!(
        enriched.access_group_models,
        Some(vec!["gpt-4o".to_owned()])
    );
    assert_eq!(enriched.access_group_mcp_server_ids, None);
    assert_eq!(
        enriched.access_group_agent_ids,
        Some(vec!["agent-1".to_owned()])
    );

    let parsed = TeamListResponse::from_value(&union_document()).unwrap();
    assert_eq!(parsed.teams.len(), 3);
    assert!(matches!(&parsed.teams[0], TeamListEntry::Enriched(_)));
    assert!(matches!(&parsed.teams[1], TeamListEntry::Enriched(_)));
    assert!(matches!(&parsed.teams[2], TeamListEntry::Deleted(_)));
}

#[test]
fn list_member_defaults_and_audit_fields() {
    let parsed = TeamListResponse::from_value(&union_document()).unwrap();
    let TeamListEntry::Enriched(live) = &parsed.teams[1] else {
        panic!("expected the enriched member");
    };
    assert_eq!(live.team.fields.base.team_id, "team-bare");
    assert_eq!(live.members_count, 0);
    assert_eq!(live.keys_count, 0);
    let TeamListEntry::Deleted(deleted) = &parsed.teams[2] else {
        panic!("expected the deleted member");
    };
    assert_eq!(deleted.team.fields.base.team_id, "team-gone");
    assert_eq!(deleted.audit.deleted_by.as_deref(), Some("admin"));
    assert!(deleted.audit.deleted_at.is_some());
    assert_eq!(
        object(&to_data(deleted).unwrap()).get("members_count"),
        None
    );
}

#[test]
fn list_union_scores_declared_keys() {
    let scored = TeamListResponse::from_value(&parse(
        r#"{"teams": [
            {"team_id": "a", "deleted_at": null},
            {"team_id": "b", "deleted_at": null, "members_count": 1},
            {"team_id": "c", "deleted_by": "admin", "id": "row"},
            {"team_id": "d", "members_count": 1, "keys_count": 2, "deleted_at": null},
            {"team_id": "e", "members_count": 1, "deleted_at": null, "deleted_by": "admin"}
        ], "total": 5, "page": 1, "page_size": 5, "total_pages": 1}"#,
    ))
    .unwrap();
    assert!(matches!(&scored.teams[0], TeamListEntry::Deleted(_)));
    assert!(matches!(&scored.teams[1], TeamListEntry::Enriched(_)));
    assert!(matches!(&scored.teams[2], TeamListEntry::Deleted(_)));
    assert!(matches!(&scored.teams[3], TeamListEntry::Enriched(_)));
    assert!(matches!(&scored.teams[4], TeamListEntry::Deleted(_)));
}

#[test]
fn list_response_serializes_union_members() {
    let response = TeamListResponse {
        teams: vec![
            TeamListEntry::Enriched(TeamListItem::try_from(&parse(ENRICHED_ROW)).unwrap()),
            TeamListEntry::Live(LiteLLMTeamTable::try_from(parse(LIVE_ROW)).unwrap()),
            TeamListEntry::Deleted(LiteLLMDeletedTeamTable::try_from(parse(DELETED_ROW)).unwrap()),
        ],
        total: 3,
        page: 2,
        page_size: 1,
        total_pages: 2,
    };
    let encoded = to_data(&response).unwrap();
    let entries = object(&encoded);
    let JsonData::Array(items) = entries.get("teams").unwrap() else {
        panic!("expected an array");
    };
    assert_eq!(items.len(), 3);
    let enriched_fields = object(&items[0]);
    assert_eq!(
        enriched_fields.get("team_id"),
        Some(&parse(r#""team-live""#))
    );
    assert_eq!(enriched_fields.get("members_count"), Some(&parse("3")));
    assert_eq!(enriched_fields.get("keys_count"), Some(&parse("2")));
    assert_eq!(
        enriched_fields.get("access_group_models"),
        Some(&parse(r#"["gpt-4o"]"#))
    );
    assert_eq!(
        enriched_fields.get("access_group_mcp_server_ids"),
        Some(&JsonData::Null)
    );
    assert_eq!(
        enriched_fields.get("access_group_agent_ids"),
        Some(&parse(r#"["agent-1"]"#))
    );
}

#[test]
fn list_response_serializes_pagination() {
    let response = TeamListResponse {
        teams: Vec::new(),
        total: 3,
        page: 2,
        page_size: 1,
        total_pages: 2,
    };
    let entries = to_data(&response).unwrap();
    let fields = object(&entries);
    assert_eq!(fields.get("total"), Some(&parse("3")));
    assert_eq!(fields.get("page"), Some(&parse("2")));
    assert_eq!(fields.get("page_size"), Some(&parse("1")));
    assert_eq!(fields.get("total_pages"), Some(&parse("2")));
}

#[test]
fn list_response_serializes_bare_and_deleted_members() {
    let response = TeamListResponse {
        teams: vec![
            TeamListEntry::Live(LiteLLMTeamTable::try_from(parse(LIVE_ROW)).unwrap()),
            TeamListEntry::Deleted(LiteLLMDeletedTeamTable::try_from(parse(DELETED_ROW)).unwrap()),
        ],
        total: 2,
        page: 1,
        page_size: 2,
        total_pages: 1,
    };
    let encoded = to_data(&response).unwrap();
    let entries = object(&encoded);
    let JsonData::Array(items) = entries.get("teams").unwrap() else {
        panic!("expected an array");
    };
    let live_fields = object(&items[0]);
    assert_eq!(live_fields.get("team_id"), Some(&parse(r#""team-bare""#)));
    assert!(!live_fields.contains_key("members_count"));
    assert!(!live_fields.contains_key("keys_count"));
    assert!(!live_fields.contains_key("deleted_at"));
    let deleted_fields = object(&items[1]);
    assert_eq!(
        deleted_fields.get("team_id"),
        Some(&parse(r#""team-gone""#))
    );
    assert_eq!(deleted_fields.get("deleted_by"), Some(&parse(r#""admin""#)));
    assert!(deleted_fields.contains_key("deleted_at"));
    assert!(!deleted_fields.contains_key("members_count"));
}

#[test]
fn list_count_defaults() {
    let bare = TeamListItem::try_from(&parse(r#"{"team_id": "t"}"#)).unwrap();
    assert_eq!(bare.members_count, 0);
    assert_eq!(bare.keys_count, 0);
    assert_eq!(bare.access_group_models, None);
    assert_eq!(bare.access_group_mcp_server_ids, None);
    assert_eq!(bare.access_group_agent_ids, None);
    let explicit = TeamListItem::try_from(&parse(
        r#"{"team_id": "t", "members_count": -2, "keys_count": 8, "access_group_models": null}"#,
    ))
    .unwrap();
    assert_eq!(explicit.members_count, -2);
    assert_eq!(explicit.keys_count, 8);
    assert_eq!(explicit.access_group_models, None);
}

#[test]
fn list_item_rejects_invalid_documents() {
    TeamListItem::try_from(&parse(r#"{"team_alias": "t"}"#)).unwrap_err();
    TeamListItem::try_from(&parse(r#"{"team_id": "t", "members_count": 1.5}"#)).unwrap_err();
}

#[test]
fn list_entry_falls_back_to_live() {
    let boolean_count =
        TeamListEntry::from_value(&parse(r#"{"team_id": "t", "keys_count": true}"#));
    assert!(matches!(boolean_count, Ok(TeamListEntry::Live(_))));
    let fractional_count =
        TeamListEntry::from_value(&parse(r#"{"team_id": "t", "members_count": 1.5}"#));
    assert!(matches!(fractional_count, Ok(TeamListEntry::Live(_))));
    let text_count = TeamListEntry::from_value(&parse(r#"{"team_id": "t", "members_count": "3"}"#));
    assert!(matches!(text_count, Ok(TeamListEntry::Live(_))));
    let deleted_with_count = TeamListEntry::from_value(&parse(
        r#"{"team_id": "t", "members_count": 1.5, "deleted_at": null}"#,
    ));
    assert!(matches!(deleted_with_count, Ok(TeamListEntry::Deleted(_))));
    let wrong_audit_type =
        TeamListEntry::from_value(&parse(r#"{"team_id": "t", "deleted_by": 5, "id": "row"}"#));
    assert!(matches!(wrong_audit_type, Ok(TeamListEntry::Enriched(_))));
    let numeric_timestamp =
        TeamListEntry::from_value(&parse(r#"{"team_id": "t", "deleted_at": 5}"#));
    assert!(matches!(numeric_timestamp, Ok(TeamListEntry::Enriched(_))));
    TeamListEntry::from_value(&parse(r#"{"team_id": 5}"#)).unwrap_err();
}

#[test]
fn list_response_accepts_nonpositive_pagination() {
    let response = TeamListResponse::from_value(&parse(
        r#"{"teams": [], "total": 0, "page": 1, "page_size": 0, "total_pages": 0}"#,
    ))
    .unwrap();
    assert_eq!(response.teams, Vec::new());
    let negative = TeamListResponse::from_value(&parse(
        r#"{"teams": [], "total": -1, "page": -2, "page_size": -3, "total_pages": -4}"#,
    ))
    .unwrap();
    assert_eq!(negative.total, -1);
    assert_eq!(negative.page, -2);
    assert_eq!(negative.page_size, -3);
    assert_eq!(negative.total_pages, -4);
}

#[test]
fn list_response_rejects_invalid_documents() {
    TeamListResponse::from_value(&parse(
        r#"{"teams": [], "total": 0, "page": 1, "page_size": 0}"#,
    ))
    .unwrap_err();
    TeamListResponse::from_value(&parse(
        r#"{"teams": null, "total": 0, "page": 1, "page_size": 0, "total_pages": 0}"#,
    ))
    .unwrap_err();
    TeamListResponse::from_value(&parse(
        r#"{"teams": [1], "total": 0, "page": 1, "page_size": 0, "total_pages": 0}"#,
    ))
    .unwrap_err();
    TeamListResponse::from_value(&parse(
        r#"{"teams": [{"team_id": "t"}], "total": "0", "page": 1, "page_size": 0, "total_pages": 0}"#,
    ))
    .unwrap_err();
}

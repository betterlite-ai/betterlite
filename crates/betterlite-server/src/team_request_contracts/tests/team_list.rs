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

#[test]
fn list_live_deleted_enriched_union() {
    let enriched = TeamListItem::from_value(&parse(ENRICHED_ROW)).unwrap();
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

    let document = format!(
        r#"{{"teams": [{ENRICHED_ROW}, {LIVE_ROW}, {DELETED_ROW}], "total": 3, "page": 1, "page_size": 10, "total_pages": 1}}"#
    );
    let parsed = TeamListResponse::from_value(&parse(&document)).unwrap();
    assert_eq!(parsed.teams.len(), 3);
    let TeamListEntry::Enriched(first) = &parsed.teams[0] else {
        panic!("expected the enriched member");
    };
    assert_eq!(first.members_count, 3);
    let TeamListEntry::Enriched(second) = &parsed.teams[1] else {
        panic!("expected the enriched member");
    };
    assert_eq!(second.team.fields.base.team_id, "team-bare");
    assert_eq!(second.members_count, 0);
    assert_eq!(second.keys_count, 0);
    let TeamListEntry::Deleted(third) = &parsed.teams[2] else {
        panic!("expected the deleted member");
    };
    assert_eq!(third.team.fields.base.team_id, "team-gone");
    assert_eq!(third.audit.deleted_by.as_deref(), Some("admin"));
    assert!(third.audit.deleted_at.is_some());
    assert_eq!(object(&to_data(third).unwrap()).get("members_count"), None);

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

    let response = TeamListResponse {
        teams: vec![
            TeamListEntry::Enriched(enriched),
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
    assert_eq!(entries.get("total"), Some(&parse("3")));
    assert_eq!(entries.get("page"), Some(&parse("2")));
    assert_eq!(entries.get("page_size"), Some(&parse("1")));
    assert_eq!(entries.get("total_pages"), Some(&parse("2")));
    let JsonData::Array(items) = entries.get("teams").unwrap() else {
        panic!("expected an array");
    };
    assert_eq!(items.len(), 3);
    let first = object(&items[0]);
    assert_eq!(first.get("team_id"), Some(&parse(r#""team-live""#)));
    assert_eq!(first.get("members_count"), Some(&parse("3")));
    assert_eq!(first.get("keys_count"), Some(&parse("2")));
    assert_eq!(
        first.get("access_group_models"),
        Some(&parse(r#"["gpt-4o"]"#))
    );
    assert_eq!(
        first.get("access_group_mcp_server_ids"),
        Some(&JsonData::Null)
    );
    assert_eq!(
        first.get("access_group_agent_ids"),
        Some(&parse(r#"["agent-1"]"#))
    );
    let second = object(&items[1]);
    assert_eq!(second.get("team_id"), Some(&parse(r#""team-bare""#)));
    assert!(!second.contains_key("members_count"));
    assert!(!second.contains_key("keys_count"));
    assert!(!second.contains_key("deleted_at"));
    let third = object(&items[2]);
    assert_eq!(third.get("team_id"), Some(&parse(r#""team-gone""#)));
    assert_eq!(third.get("deleted_by"), Some(&parse(r#""admin""#)));
    assert!(third.contains_key("deleted_at"));
    assert!(!third.contains_key("members_count"));
}

#[test]
fn list_count_defaults() {
    let bare = TeamListItem::from_value(&parse(r#"{"team_id": "t"}"#)).unwrap();
    assert_eq!(bare.members_count, 0);
    assert_eq!(bare.keys_count, 0);
    assert_eq!(bare.access_group_models, None);
    assert_eq!(bare.access_group_mcp_server_ids, None);
    assert_eq!(bare.access_group_agent_ids, None);
    let explicit = TeamListItem::from_value(&parse(
        r#"{"team_id": "t", "members_count": -2, "keys_count": 8, "access_group_models": null}"#,
    ))
    .unwrap();
    assert_eq!(explicit.members_count, -2);
    assert_eq!(explicit.keys_count, 8);
    assert_eq!(explicit.access_group_models, None);
    assert!(TeamListItem::from_value(&parse(r#"{"team_alias": "t"}"#)).is_err());
    assert!(TeamListItem::from_value(&parse(r#"{"team_id": "t", "members_count": 1.5}"#)).is_err());

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
    assert!(TeamListEntry::from_value(&parse(r#"{"team_id": 5}"#)).is_err());

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
    assert!(
        TeamListResponse::from_value(&parse(
            r#"{"teams": [], "total": 0, "page": 1, "page_size": 0}"#
        ))
        .is_err()
    );
    assert!(
        TeamListResponse::from_value(&parse(
            r#"{"teams": null, "total": 0, "page": 1, "page_size": 0, "total_pages": 0}"#
        ))
        .is_err()
    );
    assert!(
        TeamListResponse::from_value(&parse(
            r#"{"teams": [1], "total": 0, "page": 1, "page_size": 0, "total_pages": 0}"#
        ))
        .is_err()
    );
    assert!(TeamListResponse::from_value(&parse(
        r#"{"teams": [{"team_id": "t"}], "total": "0", "page": 1, "page_size": 0, "total_pages": 0}"#
    ))
    .is_err());
}

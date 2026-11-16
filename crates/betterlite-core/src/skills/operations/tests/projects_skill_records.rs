use betterlite_db::managed_resource_rows::skills::SkillTable;

use crate::skills::store::SkillPage;

use super::support::{self, Effect, ScriptedRepository, logged, number, object};

fn row_with_id(skill_id: &str) -> SkillTable {
    let mut row = support::stored_row("alice");
    row.skill_id = skill_id.to_owned();
    row
}

fn page_of(repository: &ScriptedRepository, rows: Vec<SkillTable>) {
    repository.update(|state| state.page = rows);
}

#[test]
fn projects_skill_records_with_empty_dates_for_missing_ones() {
    support::runtime().block_on(async {
        let mut row = support::stored_row("alice");
        row.created_at = None;
        row.updated_at = None;
        let repository = ScriptedRepository::with_row(row);
        let ops = support::ops(repository);
        let mut logging = support::logging_state();
        let target = support::target(&mut logging, support::SKILL_ID);
        let skill = ops.get_skill(target).await.unwrap();
        assert_eq!(skill.created_at, "");
        assert_eq!(skill.updated_at, "");
    });
}

#[test]
fn projects_skill_records_with_iso_dates_padded_to_microseconds() {
    support::runtime().block_on(async {
        let mut row = support::stored_row("alice");
        row.created_at = Some(support::timestamped("2026-01-01T00:00:00.123"));
        row.updated_at = Some(support::timestamped("2026-01-01T00:00:00"));
        let ops = support::ops(ScriptedRepository::with_row(row));
        let mut logging = support::logging_state();
        let target = support::target(&mut logging, support::SKILL_ID);
        let skill = ops.get_skill(target).await.unwrap();
        assert_eq!(skill.created_at, "2026-01-01T00:00:00.123000");
        assert_eq!(skill.updated_at, "2026-01-01T00:00:00");
    });
}

#[test]
fn projects_skill_records_with_a_custom_source_when_the_row_has_none() {
    support::runtime().block_on(async {
        let mut row = support::stored_row("alice");
        row.source = String::new();
        let ops = support::ops(ScriptedRepository::with_row(row));
        let mut logging = support::logging_state();
        let target = support::target(&mut logging, support::SKILL_ID);
        let skill = ops.get_skill(target).await.unwrap();
        assert_eq!(skill.source, "custom");
    });
}

#[test]
fn projects_skill_records_with_the_row_source_when_it_is_set() {
    support::runtime().block_on(async {
        let mut row = support::stored_row("alice");
        row.source = "anthropic".to_owned();
        let ops = support::ops(ScriptedRepository::with_row(row));
        let mut logging = support::logging_state();
        let target = support::target(&mut logging, support::SKILL_ID);
        let skill = ops.get_skill(target).await.unwrap();
        assert_eq!(skill.source, "anthropic");
    });
}

#[test]
fn projects_skill_records_keeping_title_version_and_description() {
    support::runtime().block_on(async {
        let mut row = support::stored_row("alice");
        row.display_title = Some("Title".to_owned());
        row.latest_version = Some("2".to_owned());
        row.description = Some("Desc".to_owned());
        let ops = support::ops(ScriptedRepository::with_row(row));
        let mut logging = support::logging_state();
        let target = support::target(&mut logging, support::SKILL_ID);
        let skill = ops.get_skill(target).await.unwrap();
        assert_eq!(skill.id, support::stored_row("alice").skill_id);
        assert_eq!(skill.display_title.as_deref(), Some("Title"));
        assert_eq!(skill.latest_version.as_deref(), Some("2"));
        assert_eq!(skill.description.as_deref(), Some("Desc"));
        assert_eq!(skill.r#type, "skill");
        assert_eq!(skill.search_score, None);
    });
}

#[test]
fn projects_skill_records_in_repository_order_with_more_at_the_limit() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let first = row_with_id("litellm_skill_a");
        let second = row_with_id("litellm_skill_b");
        page_of(&repository, vec![first, second]);
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let page = SkillPage {
            limit: 2,
            offset: 0,
        };
        let response = ops
            .list_skills(&mut logging, page, None, None)
            .await
            .unwrap();
        let ids: Vec<&str> = response
            .data
            .iter()
            .map(|skill| skill.id.as_str())
            .collect();
        assert_eq!(ids, ["litellm_skill_a", "litellm_skill_b"]);
        assert!(response.has_more);
        assert_eq!(response.next_page, None);
        assert_eq!(
            repository.effects(),
            vec![Effect::FindMany {
                page,
                created_by: None
            }]
        );
    });
}

#[test]
fn projects_skill_records_with_no_more_below_the_limit() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        page_of(
            &repository,
            vec![
                row_with_id("litellm_skill_a"),
                row_with_id("litellm_skill_b"),
            ],
        );
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let page = SkillPage {
            limit: 3,
            offset: 0,
        };
        let response = ops
            .list_skills(&mut logging, page, None, None)
            .await
            .unwrap();
        assert_eq!(response.data.len(), 2);
        assert!(!response.has_more);
        assert_eq!(response.next_page, None);
    });
}

#[test]
fn projects_skill_records_with_the_default_twenty_and_zero() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let response = ops
            .list_skills(&mut logging, SkillPage::default(), None, None)
            .await
            .unwrap();
        assert!(!response.has_more);
        assert_eq!(
            repository.effects(),
            vec![Effect::FindMany {
                page: SkillPage {
                    limit: 20,
                    offset: 0
                },
                created_by: None,
            }]
        );
        assert_eq!(
            logged(&logging, "optional_params"),
            object(&[("limit", number(20)), ("offset", number(0))])
        );
    });
}

#[test]
fn projects_skill_records_with_more_for_a_zero_limit() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let ops = support::ops(repository);
        let mut logging = support::logging_state();
        let page = SkillPage {
            limit: 0,
            offset: 0,
        };
        let response = ops
            .list_skills(&mut logging, page, None, None)
            .await
            .unwrap();
        assert!(response.data.is_empty());
        assert!(response.has_more);
    });
}

#[test]
fn projects_skill_records_with_more_for_a_negative_limit() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let ops = support::ops(repository);
        let mut logging = support::logging_state();
        let page = SkillPage {
            limit: -1,
            offset: 0,
        };
        let response = ops
            .list_skills(&mut logging, page, None, None)
            .await
            .unwrap();
        assert!(response.data.is_empty());
        assert!(response.has_more);
    });
}

#[test]
fn projects_skill_records_deleted_with_the_default_type() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::with_row(support::stored_row("alice"));
        let ops = support::ops(repository);
        let mut logging = support::logging_state();
        let skill_id = support::stored_row("alice").skill_id;
        let target = support::target(&mut logging, &skill_id);
        let deleted = ops.delete_skill(target).await.unwrap();
        assert_eq!(deleted.id, skill_id);
        assert_eq!(deleted.r#type, "skill_deleted");
    });
}

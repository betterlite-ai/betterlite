use betterlite_db::postgres_store::rows::{Key, KeyAction, KeyColumn, Table};

use crate::auth_tag_budgets::rows::group::ModelAccessGroupBudgetTable;
use crate::auth_tag_budgets::rows::tag::TagBudgetTable;

const TAG_FIND: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('models', COALESCE(to_jsonb(entry.\"models\"), '[]'::jsonb), 'spend', COALESCE(to_jsonb(entry.\"spend\"), '0')))::text FROM \"LiteLLM_TagTable\" AS entry WHERE entry.\"tag_name\" = $1";
const TAG_DELETE: &str = "DELETE FROM \"LiteLLM_TagTable\" AS entry WHERE entry.\"tag_name\" = $1 RETURNING to_jsonb(entry)::text";
const GROUP_FIND: &str = "SELECT to_jsonb(entry)::text FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry WHERE entry.\"access_group_name\" = $1";

#[test]
fn keyed_queries_use_the_row_statements() {
    let tag_table = TagBudgetTable;
    let tag_name = "tag-a".to_owned();
    let tag_key = Key {
        column: KeyColumn::Id,
        value: &tag_name,
    };
    let tag_query = Table::keyed(&tag_table, KeyAction::Find, tag_key);
    assert_eq!(tag_query.sql, TAG_FIND);
    assert_eq!(tag_query.parameters.len(), 1);
    let delete_key = Key {
        column: KeyColumn::Id,
        value: &tag_name,
    };
    let delete_query = Table::keyed(&tag_table, KeyAction::Delete, delete_key);
    assert_eq!(delete_query.sql, TAG_DELETE);
    let group_table = ModelAccessGroupBudgetTable;
    let group_name = "group-a".to_owned();
    let group_key = Key {
        column: KeyColumn::Id,
        value: &group_name,
    };
    let group_query = Table::keyed(&group_table, KeyAction::Find, group_key);
    assert_eq!(group_query.sql, GROUP_FIND);
}

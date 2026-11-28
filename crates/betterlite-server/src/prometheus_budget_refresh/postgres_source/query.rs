use chrono::Utc;

use crate::key_query_plan::filters::{KeyFilters, TextMatching, UI_SESSION_TOKEN_TEAM_ID};
use crate::key_query_plan::plan::KeyListQuery;
use crate::prometheus_budget_refresh::source::BudgetPageError;

// LiteLLM's _list_key_helper call: no filters of its own, the UI session team excluded, full records
pub(super) fn key_query(page_size: u64, page: u64) -> Result<KeyListQuery, BudgetPageError> {
    let Ok(page_number) = i64::try_from(page) else {
        return Err(BudgetPageError::Window { page, page_size });
    };
    let Ok(size) = i64::try_from(page_size) else {
        return Err(BudgetPageError::Window { page, page_size });
    };
    Ok(KeyListQuery {
        now: Utc::now(),
        page: page_number,
        size,
        filters: KeyFilters {
            user_id: None,
            team_id: None,
            organization_id: None,
            key_alias: None,
            key_hash: None,
            exclude_team_id: Some(UI_SESSION_TOKEN_TEAM_ID.to_owned()),
            admin_team_ids: None,
            member_team_ids: None,
            include_created_by_keys: false,
            project_id: None,
            access_group_id: None,
            agent_id: None,
            user_matching: TextMatching::Exact,
            key_alias_matching: TextMatching::Exact,
            expires_filter: None,
            search: None,
            status_filter: None,
        },
        sort_by: None,
        sort_order: "desc".to_owned(),
        expand: Vec::new(),
        return_full_object: true,
    })
}

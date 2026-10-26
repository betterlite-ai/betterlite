#![cfg(test)]

use crate::management_routes::INTERNAL_USER_BASE_ROUTES;
use rstest::rstest;

#[rstest]
#[case::global_activity(0, "/global/activity")]
#[case::global_activity_model(1, "/global/activity/model")]
#[case::global_activity_cache_hits(2, "/global/activity/cache_hits")]
#[case::tag_daily_activity(3, "/tag/daily/activity")]
#[case::tag_daily_activity_aggregated(4, "/tag/daily/activity/aggregated")]
#[case::tag_keys(5, "/tag/daily/activity/aggregated/keys")]
#[case::tag_search(6, "/tag/daily/activity/aggregated/search")]
#[case::tag_model_top_keys(7, "/tag/daily/activity/aggregated/model_top_keys")]
#[case::tag_daily_activity_export(8, "/tag/daily/activity/export")]
#[case::tag_list(9, "/tag/list")]
#[case::v1_models_model_id(10, "/v1/models/{model_id}")]
#[case::models_model_id(11, "/models/{model_id}")]
#[case::guardrails_list(12, "/guardrails/list")]
#[case::v2_guardrails_list(13, "/v2/guardrails/list")]
#[case::project_list(14, "/project/list")]
#[case::project_info(15, "/project/info")]
#[case::search_tools_list(16, "/search_tools/list")]
#[case::tools_available_providers(17, "/search_tools/ui/available_providers")]
fn literal_cases(#[case] index: usize, #[case] literal: &str) {
    assert_eq!(INTERNAL_USER_BASE_ROUTES.get(index), Some(&literal));
}

#[test]
fn complete_table_length() {
    assert_eq!(INTERNAL_USER_BASE_ROUTES.len(), 18);
}

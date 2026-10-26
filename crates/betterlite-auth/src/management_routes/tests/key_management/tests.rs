#![cfg(test)]

use crate::management_routes::KEY_MANAGEMENT_ROUTES;
use rstest::rstest;

#[rstest]
#[case::key_generate(0, "/key/generate")]
#[case::key_update(1, "/key/update")]
#[case::key_delete(2, "/key/delete")]
#[case::key_info(3, "/key/info")]
#[case::key_regenerate(4, "/key/regenerate")]
#[case::key_service_account_generate(5, "/key/service-account/generate")]
#[case::key_key_id_regenerate(6, "/key/{key_id}/regenerate")]
#[case::key_list(7, "/key/list")]
#[case::key_block(8, "/key/block")]
#[case::key_unblock(9, "/key/unblock")]
#[case::key_bulk_update(10, "/key/bulk_update")]
#[case::team_key_bulk_update(11, "/team/key/bulk_update")]
#[case::team_daily_activity(12, "/team/daily/activity")]
#[case::team_daily_activity_aggregated(13, "/team/daily/activity/aggregated")]
#[case::team_keys(14, "/team/daily/activity/aggregated/keys")]
#[case::team_search(15, "/team/daily/activity/aggregated/search")]
#[case::team_model_top_keys(16, "/team/daily/activity/aggregated/model_top_keys")]
#[case::team_daily_activity_export(17, "/team/daily/activity/export")]
#[case::spend_logs(18, "/spend/logs")]
#[case::spend_logs_v2(19, "/spend/logs/v2")]
#[case::key_key_id_reset_spend(20, "/key/{key_id}/reset_spend")]
#[case::key_aliases(21, "/key/aliases")]
#[case::key_access_group_assignment(22, "/key/access_group_assignment")]
#[case::auto_router_manage(23, "/auto_router/manage")]
fn literal_cases(#[case] index: usize, #[case] literal: &str) {
    assert_eq!(KEY_MANAGEMENT_ROUTES.get(index), Some(&literal));
}

#[test]
fn complete_table_length() {
    assert_eq!(KEY_MANAGEMENT_ROUTES.len(), 24);
}

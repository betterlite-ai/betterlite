#![cfg(test)]

use crate::management_routes::key_routes::KEY_ENUM;
use rstest::rstest;

#[rstest]
#[case::key_generate(0, "KEY_GENERATE", "/key/generate")]
#[case::key_update(1, "KEY_UPDATE", "/key/update")]
#[case::key_delete(2, "KEY_DELETE", "/key/delete")]
#[case::key_regenerate(3, "KEY_REGENERATE", "/key/regenerate")]
#[case::key_generate_service_account(
    4,
    "KEY_GENERATE_SERVICE_ACCOUNT",
    "/key/service-account/generate"
)]
#[case::key_regenerate_with_path_param(
    5,
    "KEY_REGENERATE_WITH_PATH_PARAM",
    "/key/{key_id}/regenerate"
)]
#[case::key_block(6, "KEY_BLOCK", "/key/block")]
#[case::key_unblock(7, "KEY_UNBLOCK", "/key/unblock")]
#[case::key_bulk_update(8, "KEY_BULK_UPDATE", "/key/bulk_update")]
#[case::team_key_bulk_update(9, "TEAM_KEY_BULK_UPDATE", "/team/key/bulk_update")]
#[case::key_reset_spend(10, "KEY_RESET_SPEND", "/key/{key_id}/reset_spend")]
#[case::key_access_group_assignment(
    11,
    "KEY_ACCESS_GROUP_ASSIGNMENT",
    "/key/access_group_assignment"
)]
#[case::auto_router_manage(12, "AUTO_ROUTER_MANAGE", "/auto_router/manage")]
#[case::key_info(13, "KEY_INFO", "/key/info")]
#[case::key_health(14, "KEY_HEALTH", "/key/health")]
#[case::key_list(15, "KEY_LIST", "/key/list")]
#[case::key_aliases(16, "KEY_ALIASES", "/key/aliases")]
#[case::team_daily_activity(17, "TEAM_DAILY_ACTIVITY", "/team/daily/activity")]
#[case::team_daily_activity_aggregated(
    18,
    "TEAM_DAILY_ACTIVITY_AGGREGATED",
    "/team/daily/activity/aggregated"
)]
#[case::spend_logs(19, "SPEND_LOGS", "/spend/logs")]
#[case::spend_logs_v2(20, "SPEND_LOGS_V2", "/spend/logs/v2")]
fn enum_pairs(#[case] index: usize, #[case] name: &str, #[case] value: &str) {
    let route = &KEY_ENUM[index];
    assert_eq!(serde_json::to_value(route).unwrap(), name);
    assert_eq!(route.value(), value);
}

#[test]
fn complete_enum() {
    assert_eq!(KEY_ENUM.len(), 21);
}

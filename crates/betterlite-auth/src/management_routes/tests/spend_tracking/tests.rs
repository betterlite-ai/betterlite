#![cfg(test)]

use crate::management_routes::SPEND_TRACKING_ROUTES;
use rstest::rstest;

#[rstest]
#[case::spend_keys(0, "/spend/keys")]
#[case::spend_users(1, "/spend/users")]
#[case::spend_tags(2, "/spend/tags")]
#[case::spend_calculate(3, "/spend/calculate")]
#[case::spend_logs(4, "/spend/logs")]
#[case::spend_logs_v2(5, "/spend/logs/v2")]
#[case::spend_logs_ui(6, "/spend/logs/ui")]
#[case::spend_logs_ui_request_id(7, "/spend/logs/ui/{request_id}")]
#[case::spend_logs_session_ui(8, "/spend/logs/session/ui")]
#[case::key_spend_report(9, "/key/spend/report")]
#[case::user_spend_report(10, "/user/spend/report")]
#[case::team_spend_report(11, "/team/spend/report")]
#[case::organization_spend_report(12, "/organization/spend/report")]
#[case::spend_logs_end_users(13, "/management/v1/spend_logs/end_users")]
#[case::management_v1_spend_logs_users(14, "/management/v1/spend_logs/users")]
#[case::cost_estimate(15, "/cost/estimate")]
fn literal_cases(#[case] index: usize, #[case] literal: &str) {
    assert_eq!(SPEND_TRACKING_ROUTES.get(index), Some(&literal));
}

#[test]
fn complete_table_length() {
    assert_eq!(SPEND_TRACKING_ROUTES.len(), 16);
}

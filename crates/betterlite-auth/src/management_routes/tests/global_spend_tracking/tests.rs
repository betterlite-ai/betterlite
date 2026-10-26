#![cfg(test)]

use crate::management_routes::GLOBAL_SPEND_TRACKING_ROUTES;
use rstest::rstest;

#[rstest]
#[case::global_spend_logs(0, "/global/spend/logs")]
#[case::global_spend(1, "/global/spend")]
#[case::global_spend_keys(2, "/global/spend/keys")]
#[case::global_spend_teams(3, "/global/spend/teams")]
#[case::global_spend_end_users(4, "/global/spend/end_users")]
#[case::global_spend_models(5, "/global/spend/models")]
#[case::global_predict_spend_logs(6, "/global/predict/spend/logs")]
#[case::global_spend_report(7, "/global/spend/report")]
#[case::global_spend_provider(8, "/global/spend/provider")]
#[case::global_spend_tags(9, "/global/spend/tags")]
#[case::global_spend_all_tag_names(10, "/global/spend/all_tag_names")]
#[case::spend_capture_rate(11, "/spend/capture_rate")]
fn literal_cases(#[case] index: usize, #[case] literal: &str) {
    assert_eq!(GLOBAL_SPEND_TRACKING_ROUTES.get(index), Some(&literal));
}

#[test]
fn complete_table_length() {
    assert_eq!(GLOBAL_SPEND_TRACKING_ROUTES.len(), 12);
}

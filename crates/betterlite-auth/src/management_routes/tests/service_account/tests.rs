#![cfg(test)]

use crate::management_routes::TEAM_SERVICE_ACCOUNT_ROUTES;
use rstest::rstest;

#[rstest]
#[case::key_generate(0, "/key/generate")]
#[case::key_update(1, "/key/update")]
fn literal_cases(#[case] index: usize, #[case] literal: &str) {
    assert_eq!(TEAM_SERVICE_ACCOUNT_ROUTES.get(index), Some(&literal));
}

#[test]
fn complete_table_length() {
    assert_eq!(TEAM_SERVICE_ACCOUNT_ROUTES.len(), 2);
}

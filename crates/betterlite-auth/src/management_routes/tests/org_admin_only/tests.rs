#![cfg(test)]

use crate::management_routes::ORG_ADMIN_ONLY_ROUTES;
use rstest::rstest;

#[rstest]
#[case::organization_info(0, "/organization/info")]
#[case::organization_delete(1, "/organization/delete")]
#[case::organization_member_add(2, "/organization/member_add")]
#[case::organization_member_update(3, "/organization/member_update")]
#[case::organization_member_delete(4, "/organization/member_delete")]
fn literal_cases(#[case] index: usize, #[case] literal: &str) {
    assert_eq!(ORG_ADMIN_ONLY_ROUTES.get(index), Some(&literal));
}

#[test]
fn complete_table_length() {
    assert_eq!(ORG_ADMIN_ONLY_ROUTES.len(), 5);
}

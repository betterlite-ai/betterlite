#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;

#[rstest]
#[case::org_admin_disabled("helpers/org-admin-keys/org-admin-disabled")]
#[case::org_role_not_membership("helpers/org-admin-keys/org-role-not-membership")]
fn replays_b10015_against_litellm(#[case] name: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let source = fs::read_to_string(root.join(format!("cases/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
    let result = run_case(&[crate::GROUP], &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}

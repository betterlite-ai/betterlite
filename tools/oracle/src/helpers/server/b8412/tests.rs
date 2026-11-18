#![cfg(test)]

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;
use std::fs;
use std::path::Path;

#[rstest]
#[case::delete_team_duplicates("helpers/member-request-contracts/delete-team-duplicates")]
#[case::callback_invalid_variable("helpers/member-request-contracts/callback-invalid-variable")]
#[case::callback_metadata_nulls("helpers/member-request-contracts/callback-metadata-nulls")]
#[case::member_add_one_or_many("helpers/member-request-contracts/member-add-one-or-many")]
#[case::temp_budget_pair("helpers/member-request-contracts/temp-budget-pair")]
#[case::org_member_role("helpers/member-request-contracts/org-member-role")]
#[case::organization_metadata_move("helpers/member-request-contracts/organization-metadata-move")]
#[case::organization_v2_unknown_key("helpers/member-request-contracts/organization-v2-unknown-key")]
fn replays_b8412_against_litellm(#[case] name: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let source = fs::read_to_string(root.join(format!("cases/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
    let result = run_case(&[crate::GROUP], &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}

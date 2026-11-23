#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[rstest]
#[case::strict("mcp-public-servers/strict-lists-only-allowlisted")]
#[case::legacy("mcp-public-servers/legacy-adds-public-internet")]
#[case::database("mcp-public-servers/database-record-wins")]
#[case::empty_list("mcp-public-servers/explicit-empty-list")]
#[case::unknown("mcp-public-servers/unknown-listed-id")]
fn replays_b10079_public_servers_against_litellm(#[case] name: &str) {
    let root = Path::new(ORACLE_DIR);
    let source = fs::read_to_string(root.join(format!("cases/helpers/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
    let result = run_case(&[crate::GROUP], &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}

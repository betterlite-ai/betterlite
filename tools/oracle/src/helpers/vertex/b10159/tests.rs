#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;

#[rstest]
fn matches_pinned_vertex_partner_response_config(
    #[base_dir = "../../.."]
    #[files("cases/helpers/vertex-partner-response-config/*.yaml")]
    path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).unwrap(),
        HelperRun::Match
    ));
}

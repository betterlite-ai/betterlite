#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};

use super::Op;

const OPERATION: &str = "b3338";

#[rstest]
fn replays_the_video_generation_call_cases(
    #[base_dir = "../../.."]
    #[files("cases/helpers/core/video-generation-call/*.yaml")]
    path: PathBuf,
) {
    assert_eq!(replay_case(&path), Ok(()));
}

#[test]
fn rejects_an_unknown_function() {
    let input = serde_json::json!({"function": "remix"});
    let error = Op::decode_and_run(&input).expect_err("the input names an unknown function");
    assert_eq!(error.class, "InvalidInput");
}

#[test]
fn rejects_a_call_without_its_keywords() {
    let input = serde_json::json!({"function": "requested", "call": {"prompt": "a video"}});
    let error = Op::decode_and_run(&input).expect_err("the call is missing its fields");
    assert_eq!(error.class, "InvalidInput");
}

fn replay_case(path: &Path) -> Result<(), String> {
    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let case: HelperCase = serde_saphyr::from_str(&source).map_err(|error| error.to_string())?;
    validate_helper(path, &case).map_err(|error| error.to_string())?;
    if case.call.operation != OPERATION {
        return Err(format!(
            "the case names {} instead of {OPERATION}",
            case.call.operation
        ));
    }
    match run_case(&[crate::GROUP], &case, false).map_err(|error| error.to_string())? {
        HelperRun::Match => Ok(()),
        HelperRun::Difference => Err(format!("{} does not match LiteLLM", path.display())),
    }
}

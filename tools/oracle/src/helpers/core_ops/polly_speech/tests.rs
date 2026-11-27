#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};

const OPERATION: &str = "polly_speech";

#[rstest]
fn replays_the_polly_speech_cases(
    #[base_dir = "../../.."]
    #[files("cases/helpers/polly-speech/*.yaml")]
    path: PathBuf,
) {
    assert_eq!(replay_case(&path), Ok(()));
}

#[test]
fn replays_the_polly_speech_seeds() {
    let seeds = seed_paths();
    assert!(!seeds.is_empty(), "the polly_speech seeds are present");
    let failures: Vec<String> = seeds
        .iter()
        .filter_map(|path| replay_case(path).err())
        .collect();
    assert!(failures.is_empty(), "{failures:?}");
}

fn seed_paths() -> Vec<PathBuf> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fuzz/corpus/helper_case");
    yaml_paths(&corpus)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(OPERATION))
        })
        .collect()
}

fn yaml_paths(folder: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(folder)
        .expect("the folder is readable")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "yaml")
        })
        .collect();
    paths.sort();
    paths
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

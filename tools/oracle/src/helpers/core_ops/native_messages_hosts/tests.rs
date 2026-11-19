#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};

use super::{Host, Input, Step};

const OPERATION: &str = "native_messages_hosts";

#[rstest]
fn replays_the_native_messages_hosts_cases(
    #[base_dir = "../../.."]
    #[files("cases/helpers/native-messages-hosts/*.yaml")]
    path: PathBuf,
) {
    assert_eq!(replay_case(&path), Ok(()));
}

#[test]
fn replays_the_native_messages_hosts_seeds() {
    let seeds = seed_paths();
    assert!(
        !seeds.is_empty(),
        "the native_messages_hosts seed is present"
    );
    let failures: Vec<String> = seeds
        .iter()
        .filter_map(|path| replay_case(path).err())
        .collect();
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn native_messages_hosts_seed_covers_each_host() {
    let seeds = seed_paths();
    assert!(
        !seeds.is_empty(),
        "the native_messages_hosts seed is present"
    );
    let mut hosts = BTreeSet::new();
    for path in &seeds {
        let case = parse_case(path);
        assert_eq!(case.call.operation, OPERATION, "{}", path.display());
        let input: Input =
            serde_json::from_value(case.call.input.clone()).expect("the seed names a typed input");
        for step in &input.calls {
            hosts.insert(host_name(step_host(step)));
        }
    }
    let expected: BTreeSet<&str> = ["deepseek", "minimax", "tencent"].into_iter().collect();
    assert_eq!(hosts, expected);
}

fn step_host(step: &Step) -> Host {
    match step {
        Step::Url(call) => call.host,
        Step::Environment(call) => call.host,
        Step::Transform(call) => call.host,
    }
}

fn host_name(host: Host) -> &'static str {
    match host {
        Host::Deepseek => "deepseek",
        Host::Tencent => "tencent",
        Host::Minimax => "minimax",
    }
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

fn parse_case(path: &Path) -> HelperCase {
    let source = fs::read_to_string(path).expect("the case is readable");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("the case parses");
    validate_helper(path, &case).expect("the case is valid");
    case
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

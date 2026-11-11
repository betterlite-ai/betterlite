#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::compare::helper::HelperOutput;
use crate::helpers::invoke;

fn seeds() -> Vec<(String, HelperCase)> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut seeds: Vec<(String, HelperCase)> = fs::read_dir(corpus)
        .expect("the helper corpus")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("host_model_identity"))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).expect("a seed");
            let case: HelperCase = serde_saphyr::from_str(&source).expect("a helper case");
            (path.display().to_string(), case)
        })
        .collect();
    seeds.sort_by(|left, right| left.0.cmp(&right.0));
    seeds
}

#[test]
fn helper_seeds_select_host_model_identity() {
    let seeds = seeds();
    assert!(!seeds.is_empty());
    let rejected: Vec<&str> = seeds
        .iter()
        .filter(|(_, case)| {
            let decodes = serde_json::from_value::<super::Input>(case.call.input.clone()).is_ok();
            case.call.operation != "host_model_identity"
                || !decodes
                || validate_helper(Path::new("<helper>"), case).is_err()
        })
        .map(|(path, _)| path.as_str())
        .collect();
    assert!(rejected.is_empty(), "{rejected:?}");
}

fn covers(output: &HelperOutput, host: &str) -> bool {
    output
        .value
        .as_ref()
        .and_then(Value::as_array)
        .is_some_and(|steps| {
            steps.iter().any(|step| {
                step["host"] == host
                    && step["api_base"]
                        .as_str()
                        .is_some_and(|api_base| !api_base.is_empty())
            })
        })
}

#[test]
fn helper_seeds_cover_every_host_policy() {
    let outputs: Vec<HelperOutput> = seeds()
        .into_iter()
        .map(|(_, case)| invoke(&case.call))
        .collect();
    let missing: Vec<&str> = ["clarifai", "compactifai", "datarobot"]
        .into_iter()
        .filter(|host| !outputs.iter().any(|output| covers(output, host)))
        .collect();
    assert!(missing.is_empty(), "{missing:?} have no seed");
}

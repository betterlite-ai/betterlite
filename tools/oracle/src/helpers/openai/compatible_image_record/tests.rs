#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_config::sections::compatible_image_credentials::Host;

use super::Input;
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
                .is_some_and(|name| name.starts_with("compatible_image_record"))
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
fn helper_seeds_select_compatible_image_record() {
    let seeds = seeds();
    assert!(!seeds.is_empty());
    let rejected: Vec<&str> = seeds
        .iter()
        .filter(|(_, case)| {
            let decodes = serde_json::from_value::<Input>(case.call.input.clone()).is_ok();
            case.call.operation != "compatible_image_record"
                || !decodes
                || validate_helper(Path::new("<helper>"), case).is_err()
        })
        .map(|(path, _)| path.as_str())
        .collect();
    assert!(rejected.is_empty(), "{rejected:?}");
}

fn inputs() -> Vec<Input> {
    seeds()
        .iter()
        .map(|(_, case)| {
            serde_json::from_value(case.call.input.clone())
                .expect("a compatible_image_record input")
        })
        .collect()
}

fn hosts(input: &Input) -> Vec<Host> {
    match input {
        Input::Options { options } => options.iter().map(|call| call.host).collect(),
        Input::Request { requests } => requests.iter().map(|call| call.host).collect(),
        Input::Price { prices } => prices.iter().map(|call| call.host).collect(),
    }
}

#[test]
fn helper_seeds_cover_both_hosts_and_functions() {
    let inputs = inputs();
    assert!(!inputs.is_empty());
    assert!(
        inputs
            .iter()
            .any(|input| matches!(input, Input::Options { .. }))
    );
    assert!(
        inputs
            .iter()
            .any(|input| matches!(input, Input::Request { .. }))
    );
    assert!(
        inputs
            .iter()
            .any(|input| matches!(input, Input::Price { .. }))
    );
    let hosts: Vec<Host> = inputs.iter().flat_map(hosts).collect();
    assert!(hosts.contains(&Host::Aiml));
    assert!(hosts.contains(&Host::CometApi));
    let outputs: Vec<HelperOutput> = seeds().iter().map(|(_, case)| invoke(&case.call)).collect();
    assert!(outputs.iter().all(|output| output.ok), "a seed did not run");
}

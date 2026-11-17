#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::ops::Input;

#[test]
fn helper_seeds_select_b9961() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("b9961"))
        })
        .map(|path| serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|seed| seed.call.operation == "b9961"));
    assert!(
        seeds
            .iter()
            .all(|seed| { serde_json::from_value::<Input>(seed.call.input.clone()).is_ok() })
    );
}

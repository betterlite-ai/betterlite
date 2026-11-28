#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::parse_helper_case;
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use crate::GROUP;

use super::{Case, Input};

#[test]
fn helper_seeds_select_bedrock_image_dispatch() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .expect("the helper corpus")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_image_dispatch"))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).expect("a seed");
            (path, source)
        })
        .collect();
    assert!(!seeds.is_empty(), "no bedrock_image_dispatch seeds");
    assert!(seeds.iter().all(|(path, source)| {
        let case = parse_helper_case(path, source).expect("a valid helper case");
        let input: Input = serde_json::from_value(case.call.input.clone()).expect("the seed input");
        case.call.operation == "bedrock_image_dispatch"
            && matches!(input.case, Case::TitanImage)
            && (GROUP.check)(&case.call.operation, &case.call.input) == Some(Ok(()))
    }));
}

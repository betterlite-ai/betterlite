#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

// Each seed is named after the variant it selects: openai_image_edits-<variant with hyphens>.yaml.
#[test]
fn helper_seeds_select_openai_image_edits() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let mut seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter_map(|path| {
            let stem = path
                .file_stem()?
                .to_str()?
                .strip_prefix("openai_image_edits-")?;
            Some((
                stem.to_owned(),
                fs::read_to_string(path).expect("a seed is readable"),
            ))
        })
        .collect();
    seeds.sort();
    assert!(!seeds.is_empty(), "no openai_image_edits seeds");
    assert!(seeds.iter().all(|(stem, source)| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("a seed parses");
        validate_helper(Path::new("<helper>"), &case).expect("a seed validates");
        case.call.operation == "openai_image_edits"
            && case.call.input.as_object().is_some_and(|input| {
                input.len() == 1
                    && input
                        .keys()
                        .next()
                        .is_some_and(|key| key.replace('_', "-") == *stem)
            })
    }));
}

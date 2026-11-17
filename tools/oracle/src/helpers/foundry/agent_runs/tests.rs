#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

fn selects_the_operation(source: &str) -> bool {
    let Ok(case) = serde_saphyr::from_str::<HelperCase>(source) else {
        return false;
    };
    validate_helper(Path::new("<helper seed>"), &case).is_ok()
        && case.call.operation == "foundry_agent_runs"
}

#[test]
fn helper_seeds_select_foundry_agent_runs() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("the helper case corpus")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("foundry_agent_runs"))
        })
        .collect();
    assert!(
        !seeds.is_empty(),
        "the corpus holds a foundry_agent_runs seed"
    );
    let rejected: Vec<String> = seeds
        .iter()
        .filter(|path| !selects_the_operation(&fs::read_to_string(path).expect("a seed")))
        .map(|path| path.display().to_string())
        .collect();
    assert!(
        rejected.is_empty(),
        "seeds calling another operation: {rejected:?}"
    );
}

#![cfg(test)]

use std::fs;
use std::iter;
use std::path::Path;

use betterlite_oracle_runner::case::helper::parse_helper_case;
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use crate::GROUPS;

// the oracle crate's test of the same name holds every group and also rejects an operation no group lists
#[test]
fn replays_the_helper_case_corpus() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let problems: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter_map(|path| {
            let text = fs::read_to_string(&path).unwrap();
            match parse_helper_case(&path, &text) {
                Err(error) => Some(error.to_string()),
                Ok(case) => iter::once(&case.call)
                    .chain(&case.calls)
                    .find_map(|call| call.owned_check(GROUPS).and_then(Result::err))
                    .map(|message| format!("{}: {message}", path.display())),
            }
        })
        .collect();
    assert!(problems.is_empty(), "{problems:#?}");
}

#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper;
use betterlite_oracle_runner::case::helper::parse_helper_case;
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;

#[test]
fn helper_seeds_select_openai_resource_dispatch() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .expect("the helper corpus")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("openai_resource_dispatch"))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).expect("a seed");
            (path, source)
        })
        .collect();
    assert!(!seeds.is_empty(), "no openai_resource_dispatch seeds");
    seeds.iter().for_each(|(path, source)| {
        let case = parse_helper_case(path, source).expect("a valid helper case");
        assert_eq!(case.call.operation, "openai_resource_dispatch");
        <Op as Helper>::check(&case.call.input).expect("the dispatch input");
    });
}

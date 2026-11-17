#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::semantic_router::index::parse_inline;

mod semantic_router_behaviors;
mod semantic_router_boundaries;
mod support;

#[rstest]
fn replays_the_route_index_corpus(#[files("fuzz/corpus/route_index/*")] path: PathBuf) {
    let source = fs::read_to_string(&path).expect("the corpus seed is readable");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let outcome = parse_inline(&source);
    if name.contains("invalid") {
        assert!(outcome.is_err(), "{name} should be rejected");
    } else {
        let _index = outcome.unwrap_or_else(|error| panic!("{name} should parse: {error}"));
    }
}

use std::fs;
use std::path::Path;

use crate::semantic_guard::semantic_guard_route_loading::custom_routes::decode;

#[test]
fn replays_the_semantic_guard_routes_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/semantic_guard_routes");
    let outcomes: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            decode(&bytes)
        })
        .collect();
    assert!(!outcomes.is_empty());
}

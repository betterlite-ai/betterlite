use std::fs;
use std::path::Path;

use crate::lakera::response::LakeraModerationResponse;
use crate::lasso::MaskedMessages;

#[test]
fn third_party_hooks_wire_replays_the_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/third_party_hooks_wire");
    let paths: Vec<_> = fs::read_dir(corpus).expect("corpus").collect();
    assert!(!paths.is_empty());
    assert!(paths.into_iter().all(|entry| {
        let bytes = fs::read(entry.expect("entry").path()).expect("seed");
        serde_json::from_slice::<LakeraModerationResponse>(&bytes).is_ok()
            && serde_json::from_slice::<MaskedMessages>(&bytes).is_ok()
    }));
}

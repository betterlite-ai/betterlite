use std::fs;
use std::path::Path;

use crate::endpoints::passthrough_config_writes::routes::decode_body;

#[test]
fn replays_the_pass_through_config_writes_corpus() {
    assert!(replay_corpus() > 0);
}

fn replay_corpus() -> usize {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/pass_through_config_writes");
    let mut replayed = 0_usize;
    for entry in fs::read_dir(&root).expect("the corpus directory") {
        let bytes = fs::read(entry.expect("a corpus entry").path()).expect("a corpus file");
        drop(decode_body(&bytes));
        replayed = replayed.saturating_add(1);
    }
    replayed
}

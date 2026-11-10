#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use betterlite_logging::datadog_logging::settings::{DD_MAX_BATCH_SIZE, resolve_batch_size};
use rstest::rstest;

#[rstest]
fn replays_the_datadog_logs_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/datadog_logs");
    let mut seen = 0_usize;
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let bytes = fs::read(&path).unwrap();
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        seen = seen.saturating_add(1);
        let environment = BTreeMap::from([("DD_BATCH_SIZE".to_owned(), text.to_owned())]);
        let resolved = resolve_batch_size(&environment);
        assert!(
            (1..=DD_MAX_BATCH_SIZE).contains(&resolved),
            "{} resolved to {resolved}",
            path.display()
        );
    }
    assert!(seen > 0, "the datadog logs corpus is empty");
}

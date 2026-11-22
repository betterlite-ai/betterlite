#![cfg(test)]

use std::fs;

use crate::api::file_batch_guardrails::lines::iter_records;
use crate::api::file_batch_guardrails::record_json::load_record;

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fuzz/corpus/file_batch_guardrails"
);

fn seeds() -> Vec<Vec<u8>> {
    fs::read_dir(CORPUS)
        .expect("the corpus directory exists")
        .map(|entry| {
            let path = entry.expect("the corpus entry lists").path();
            fs::read(&path).unwrap_or_else(|source| panic!("{} reads: {source}", path.display()))
        })
        .collect()
}

#[test]
fn replays_corpus() {
    let seeds = seeds();
    assert!(
        seeds
            .iter()
            .any(|seed| load_record(seed.as_slice()).is_ok()),
        "no seed loads"
    );
    assert!(
        seeds
            .iter()
            .any(|seed| load_record(seed.as_slice()).is_err()),
        "no seed is refused"
    );
    assert!(
        seeds
            .iter()
            .any(|seed| iter_records(seed.as_slice()).next().is_none()),
        "no seed replays as no records"
    );
    seeds
        .iter()
        .filter_map(|seed| {
            load_record(seed.as_slice())
                .ok()
                .map(|record| (seed, record))
        })
        .for_each(|(seed, record)| {
            let mut replayed = iter_records(seed.as_slice());
            let first = replayed
                .next()
                .expect("a seed that loads yields a record")
                .expect("the record parses");
            assert_eq!(first.payload, record);
            assert!(replayed.next().is_none(), "a seed that loads is one record");
        });
}

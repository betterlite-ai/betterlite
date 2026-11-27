use std::fs;
use std::str;

use crate::batch_embeddings::classify::BatchRecordKind;
use crate::batch_records::error::BatchRecordsError;
use crate::batch_records::record::{
    CustomId, LINE_BOUNDARIES, OpenAiBatchRecord, decode_batch_input_line,
};

#[test]
fn replays_the_bedrock_services_batch_records_corpus() {
    let seeds = replay_corpus();
    assert!(seeds >= 9, "the corpus holds {seeds} seeds");
}

fn replay_corpus() -> usize {
    let directory = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fuzz/corpus/bedrock_services_batch_records"
    );
    let mut seeds = 0_usize;
    for entry in fs::read_dir(directory).expect("the corpus directory reads") {
        let path = entry.expect("the corpus entry reads").path();
        let bytes = fs::read(&path).expect("the seed reads");
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("the seed is named");
        let outcomes = splitlines_outcomes(&bytes);
        let records: Vec<&OpenAiBatchRecord> = outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().ok().and_then(Option::as_ref))
            .collect();
        match name {
            "valid-chat" => {
                assert_eq!(records.len(), 1, "the valid seed carries one record");
                assert_eq!(records[0].custom_id, CustomId::Text(String::from("a")));
                assert_eq!(records[0].kind, BatchRecordKind::Chat);
            }
            "null-custom-id" => {
                assert_eq!(records.len(), 1, "the null seed carries one record");
                assert_eq!(records[0].custom_id, CustomId::Null);
            }
            "boundary-whitespace" => {
                let ids: Vec<&CustomId> = records.iter().map(|record| &record.custom_id).collect();
                assert_eq!(
                    ids,
                    vec![
                        &CustomId::Text(String::from("b")),
                        &CustomId::Text(String::from("c")),
                        &CustomId::Text(String::from("d")),
                    ]
                );
            }
            "blank-line" => {
                assert!(
                    outcomes.iter().all(Result::is_ok),
                    "every blank row decodes to no record"
                );
                assert!(records.is_empty(), "the blank seed carries no record");
            }
            "padded-nbsp" | "padded-en-quad" | "padded-unit-separator" => {
                assert!(records.is_empty(), "the padded seed carries no record");
                assert!(
                    outcomes.iter().any(Result::is_err),
                    "json.loads reads the row as splitlines returns it, so only space and tab pad it"
                );
            }
            "malformed-json" | "truncated" => {
                assert!(
                    outcomes.iter().any(Result::is_err),
                    "the malformed seed is refused"
                );
            }
            other => panic!("unexpected seed {other}"),
        }
        seeds = seeds.saturating_add(1);
    }
    seeds
}

// batch_content splits the upload where Python's splitlines breaks, then decodes each row.
fn splitlines_outcomes(bytes: &[u8]) -> Vec<Result<Option<OpenAiBatchRecord>, BatchRecordsError>> {
    let text = str::from_utf8(bytes).expect("the seed is UTF-8");
    text.split(LINE_BOUNDARIES)
        .map(|line| decode_batch_input_line(line.as_bytes()))
        .collect()
}

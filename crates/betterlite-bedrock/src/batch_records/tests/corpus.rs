use crate::batch_embeddings::classify::BatchRecordKind;
use crate::batch_records::record::{CustomId, decode_batch_input_line};

#[test]
fn replays_the_bedrock_services_batch_records_corpus() {
    let directory = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fuzz/corpus/bedrock_services_batch_records"
    );
    let mut seeds = 0_usize;
    for entry in std::fs::read_dir(directory).expect("the corpus directory reads") {
        let path = entry.expect("the corpus entry reads").path();
        let bytes = std::fs::read(&path).expect("the seed reads");
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("the seed is named");
        let outcome = decode_batch_input_line(&bytes);
        match name {
            "valid-chat" => {
                let record = outcome
                    .expect("the valid seed decodes")
                    .expect("the valid seed is a record");
                assert_eq!(record.custom_id, CustomId::Text(String::from("a")));
                assert_eq!(record.kind, BatchRecordKind::Chat);
            }
            "null-custom-id" => {
                let record = outcome
                    .expect("the null seed decodes")
                    .expect("the null seed is a record");
                assert_eq!(record.custom_id, CustomId::Null);
            }
            "boundary-whitespace" => {
                let record = outcome
                    .expect("the boundary seed decodes")
                    .expect("the boundary seed is a record");
                assert_eq!(record.custom_id, CustomId::Text(String::from("b")));
            }
            "blank-line" => assert!(matches!(outcome, Ok(None))),
            "malformed-json" | "truncated" => {
                outcome.expect_err("the malformed seed is refused");
            }
            other => panic!("unexpected seed {other}"),
        }
        seeds = seeds.saturating_add(1);
    }
    assert!(seeds >= 6, "the corpus holds {seeds} seeds");
}

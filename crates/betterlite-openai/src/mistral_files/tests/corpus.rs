use std::fs;
use std::path::{Path, PathBuf};

use crate::api::batch_records::{BatchRecordsError, WireDecodeLimits};
use crate::mistral_batches::records::MistralBatchJob;
use crate::mistral_files::decode::{MistralDecodeError, decode_mistral_resource};
use crate::mistral_files::records::{MistralFile, MistralFileDeleted, MistralFileList};

fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0004_0000,
        max_items: 10_000,
    }
}

fn decode_tagged(bytes: &[u8]) -> Result<(), MistralDecodeError> {
    let Some((tag, body)) = bytes.split_first() else {
        return Ok(());
    };
    match tag {
        1 => decode_mistral_resource::<MistralFile>(body, &limits()).map(drop),
        2 => decode_mistral_resource::<MistralFileList>(body, &limits()).map(drop),
        3 => decode_mistral_resource::<MistralFileDeleted>(body, &limits()).map(drop),
        _ => decode_mistral_resource::<MistralBatchJob>(body, &limits()).map(drop),
    }
}

#[test]
fn replays_mistral_resource_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/mistral_resource");
    let mut paths: Vec<PathBuf> = fs::read_dir(corpus)
        .expect("the corpus folder exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "the corpus has seeds");
    for path in &paths {
        let bytes = fs::read(path).expect("the seed reads");
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the seed name is UTF-8");
        let decoded = decode_tagged(&bytes);
        if name.starts_with("valid-") {
            decoded.unwrap_or_else(|error| panic!("{name}: {error}"));
        } else {
            assert!(name.starts_with("invalid-"), "{name} is classified");
            assert!(decoded.is_err(), "{name}: {decoded:?}");
        }
    }
}

#[test]
fn the_json_limit_rejects_one_byte_over() {
    let body = br#"{"id": "f"}"#;
    let at = WireDecodeLimits {
        max_json_bytes: body.len(),
        ..limits()
    };
    let below = WireDecodeLimits {
        max_json_bytes: body.len() - 1,
        ..limits()
    };
    assert!(decode_mistral_resource::<serde_json::Value>(body, &at).is_ok());
    assert!(matches!(
        decode_mistral_resource::<serde_json::Value>(body, &below),
        Err(MistralDecodeError::Limit(BatchRecordsError::JsonTooLarge { limit }))
            if limit == body.len() - 1
    ));
}

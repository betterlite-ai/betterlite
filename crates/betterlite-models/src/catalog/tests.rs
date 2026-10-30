#![cfg(test)]

use super::{
    BundledCatalog, CatalogError, MAX_CATALOG_BYTES, MAX_CATALOG_DEPTH, MAX_CATALOG_ENTRIES,
    ModelCatalog, UnknownFields,
};
use crate::metadata::identity::Mode;
use rstest::rstest;
use std::sync::Arc;
use std::{
    fs, io,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Copy, Debug)]
enum Outcome {
    Count(usize),
    Price,
    Duplicate,
    TooLarge,
    TooDeep,
    TooMany,
    Invalid,
    Unknown,
    Warnings,
    Alias(&'static str),
    Generalizations,
    GeneralizationsEmpty,
    GeneralizationWarnings,
    GeneralizationUnknown,
}

#[rstest]
#[case::pinned_backup(fs::read("fixtures/model_prices_and_context_window_backup.json").unwrap(), UnknownFields::Reject, Outcome::Count(4458))]
#[case::pinned_root(fs::read("fixtures/model_prices_and_context_window.json").unwrap(), UnknownFields::Reject, Outcome::Count(4458))]
#[case::empty(b"{}".to_vec(), UnknownFields::Reject, Outcome::Count(0))]
#[case::canonical(br#"{"model":{"litellm_provider":"openai","mode":"audio_transcription","input_cost_per_token":0.0000001}}"#.to_vec(), UnknownFields::Reject, Outcome::Price)]
#[case::aliases(br#"{"model":{"litellm_provider":"openai","aliases":["alias"]}}"#.to_vec(), UnknownFields::Reject, Outcome::Alias("alias"))]
#[case::canonical_conflict(br#"{"first":{"aliases":["model"]},"model":{"litellm_provider":"openai"}}"#.to_vec(), UnknownFields::Reject, Outcome::Alias("model"))]
#[case::alias_conflict(br#"{"model":{"litellm_provider":"openai","aliases":["alias"]},"second":{"aliases":["alias"]}}"#.to_vec(), UnknownFields::Reject, Outcome::Alias("alias"))]
#[case::alias_source_order(br#"{"model":{"litellm_provider":"openai","aliases":["alias"]},"a-second":{"aliases":["alias"]}}"#.to_vec(), UnknownFields::Reject, Outcome::Alias("alias"))]
#[case::generalizations(br#"{"sample_spec":{},"fallback_generalizations":{"rules":[{"name":"base","pattern":"^x","model_info":{"litellm_provider":"openai"}}]},"model":{}}"#.to_vec(), UnknownFields::Reject, Outcome::Generalizations)]
#[case::generalizations_non_list(br#"{"fallback_generalizations":{"rules":"not-a-list"}}"#.to_vec(), UnknownFields::Reject, Outcome::GeneralizationsEmpty)]
#[case::generalizations_malformed_rule(br#"{"fallback_generalizations":{"rules":[{"name":"bad","pattern":5,"model_info":{}}]}}"#.to_vec(), UnknownFields::Reject, Outcome::GeneralizationWarnings)]
#[case::generalizations_unknown_model_info_warn(br#"{"fallback_generalizations":{"rules":[{"name":"base","pattern":"^x","model_info":{"new_capability":true}}]}}"#.to_vec(), UnknownFields::Warn, Outcome::GeneralizationWarnings)]
#[case::generalizations_unknown_model_info_reject(br#"{"fallback_generalizations":{"rules":[{"name":"base","pattern":"^x","model_info":{"new_capability":true}}]}}"#.to_vec(), UnknownFields::Reject, Outcome::GeneralizationUnknown)]
#[case::unknown_bundled(br#"{"model":{"new_field":true}}"#.to_vec(), UnknownFields::Reject, Outcome::Unknown)]
#[case::unknown_remote(br#"{"model":{"new_field":true,"metadata":{"new_note":"x"}}}"#.to_vec(), UnknownFields::Warn, Outcome::Warnings)]
#[case::duplicate_models(br#"{"model":{},"model":{}}"#.to_vec(), UnknownFields::Warn, Outcome::Duplicate)]
#[case::duplicate_fields(br#"{"model":{"mode":"chat","mode":"embedding"}}"#.to_vec(), UnknownFields::Warn, Outcome::Duplicate)]
#[case::duplicate_unknown(br#"{"model":{"unknown":{"key":1,"key":2}}}"#.to_vec(), UnknownFields::Warn, Outcome::Duplicate)]
#[case::duplicate_escaped(br#"{"model":{},"\u006dodel":{}}"#.to_vec(), UnknownFields::Warn, Outcome::Duplicate)]
#[case::byte_boundary(padded(MAX_CATALOG_BYTES), UnknownFields::Warn, Outcome::Count(0))]
#[case::byte_limit(padded(MAX_CATALOG_BYTES + 1), UnknownFields::Warn, Outcome::TooLarge)]
#[case::entry_boundary(
    entries(MAX_CATALOG_ENTRIES),
    UnknownFields::Warn,
    Outcome::Count(MAX_CATALOG_ENTRIES)
)]
#[case::entry_limit(entries(MAX_CATALOG_ENTRIES + 1), UnknownFields::Warn, Outcome::TooMany)]
#[case::depth_boundary(nested(MAX_CATALOG_DEPTH - 2), UnknownFields::Warn, Outcome::Count(1))]
#[case::depth_limit(nested(MAX_CATALOG_DEPTH - 1), UnknownFields::Warn, Outcome::TooDeep)]
#[case::invalid_key(br#"{"\uQQQQ":{}}"#.to_vec(), UnknownFields::Warn, Outcome::Invalid)]
#[case::unterminated(br#"{"model"#.to_vec(), UnknownFields::Warn, Outcome::Invalid)]
#[case::large_key_array(large_key_array(), UnknownFields::Warn, Outcome::Count(1))]
#[case::trailing(b"{}x".to_vec(), UnknownFields::Warn, Outcome::Invalid)]
fn model_catalog_tests_behavior(
    #[case] bytes: Vec<u8>,
    #[case] mode: UnknownFields,
    #[case] outcome: Outcome,
) {
    let result = ModelCatalog::parse(&bytes, mode);
    match outcome {
        Outcome::Count(count) => {
            let catalog = result.unwrap();
            assert_eq!(catalog.entries.len(), count);
            assert!(!catalog.entries.contains_key("sample_spec"));
            assert!(!catalog.entries.contains_key("fallback_generalizations"));
            if mode == UnknownFields::Reject {
                assert!(catalog.warnings.is_empty());
            }
        }
        Outcome::Price => {
            let catalog = result.unwrap();
            check_lookup(&catalog, "model");
            let price = catalog
                .lookup("model")
                .unwrap()
                .prices
                .values
                .input
                .input_token
                .unwrap();
            assert_eq!(price.to_string(), "0.0000001");
            assert_eq!(
                catalog.metadata("model").unwrap().mode,
                Some(Mode::AudioTranscription)
            );
        }
        Outcome::Alias(name) => check_lookup(&result.unwrap(), name),
        Outcome::Generalizations => {
            let catalog = result.unwrap();
            assert_eq!(catalog.generalizations.match_routing("x"), Some("openai"));
        }
        Outcome::GeneralizationsEmpty => {
            let catalog = result.unwrap();
            assert!(catalog.generalizations.rules.is_empty());
        }
        Outcome::GeneralizationWarnings => {
            let catalog = result.unwrap();
            assert!(!catalog.warnings.is_empty());
        }
        Outcome::GeneralizationUnknown => {
            assert!(result.unwrap_err().to_string().contains("unknown field"));
        }
        Outcome::Warnings => assert_eq!(
            result.unwrap().warnings,
            ["model.metadata.new_note", "model.new_field"]
        ),
        failure @ (Outcome::Duplicate
        | Outcome::TooLarge
        | Outcome::TooDeep
        | Outcome::TooMany
        | Outcome::Invalid
        | Outcome::Unknown) => check_error(&result.unwrap_err(), failure),
    }
}

fn check_error(error: &CatalogError, outcome: Outcome) {
    match outcome {
        Outcome::Duplicate => assert!(error.to_string().contains("duplicate")),
        Outcome::TooLarge => assert!(matches!(
            error,
            CatalogError::TooLarge {
                limit: MAX_CATALOG_BYTES,
                ..
            }
        )),
        Outcome::TooDeep => assert!(matches!(
            error,
            CatalogError::TooDeep {
                limit: MAX_CATALOG_DEPTH
            }
        )),
        Outcome::TooMany => assert!(matches!(
            error,
            CatalogError::TooManyEntries {
                limit: MAX_CATALOG_ENTRIES,
                ..
            }
        )),
        Outcome::Unknown => assert!(error.to_string().contains("unknown field")),
        Outcome::Invalid => assert!(matches!(error, CatalogError::Json(_))),
        Outcome::Price
        | Outcome::Count(_)
        | Outcome::Warnings
        | Outcome::Alias(_)
        | Outcome::Generalizations
        | Outcome::GeneralizationsEmpty
        | Outcome::GeneralizationWarnings
        | Outcome::GeneralizationUnknown => {
            panic!("expected success")
        }
    }
}

fn check_lookup(catalog: &ModelCatalog, name: &str) {
    let entry = catalog.lookup(name).unwrap();
    let metadata = catalog.metadata(name).unwrap();
    assert_eq!(entry.name, "model");
    assert_eq!(metadata.model, "model");
    assert_eq!(metadata.provider, "openai");
    assert!(catalog.lookup("missing").is_none());
    assert!(catalog.metadata("missing").is_none());
    if catalog.entries.len() > 1 {
        assert_eq!(catalog.warnings.len(), 1);
    }
}

fn padded(size: usize) -> Vec<u8> {
    let mut bytes = b"{}".to_vec();
    bytes.resize(size, b' ');
    bytes
}

fn entries(count: usize) -> Vec<u8> {
    let values = (0..count)
        .map(|index| format!("\"model-{index}\":{{}}"))
        .collect::<Vec<_>>();
    format!("{{{}}}", values.join(",")).into_bytes()
}

fn nested(depth: usize) -> Vec<u8> {
    let arrays = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    format!("{{\"model\":{{\"unknown\":{arrays}}}}}").into_bytes()
}

fn large_key_array() -> Vec<u8> {
    let key = "x".repeat(8 * 1024 * 1024);
    let array = "0,".repeat(1_000_000);
    format!("{{\"model\":{{\"{key}\":[{array}0]}}}}").into_bytes()
}

#[test]
fn bundled_map_failure_is_cached() {
    let reads = AtomicUsize::new(0);
    let mut loader = BundledCatalog {
        cached: None,
        asset: || {
            reads.fetch_add(1, Ordering::SeqCst);
            Err(CatalogError::Asset(io::Error::from(
                io::ErrorKind::NotFound,
            )))
        },
    };
    let first = loader.get();
    assert!(first.entries.is_empty());
    assert!(first.warnings.is_empty());
    assert!(Arc::ptr_eq(&first, &loader.get()));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[rstest]
#[case::bad_json(b"not-json".as_slice(), 0)]
#[case::unknown_field(br#"{"model":{"unknown":1}}"#.as_slice(), 0)]
#[case::valid(br#"{"model":{}}"#.as_slice(), 1)]
fn bundled_parse_is_cached(#[case] bytes: &[u8], #[case] count: usize) {
    let reads = AtomicUsize::new(0);
    let mut loader = BundledCatalog {
        cached: None,
        asset: || {
            reads.fetch_add(1, Ordering::SeqCst);
            Ok(bytes.to_vec())
        },
    };
    let first = loader.get();
    assert_eq!(first.entries.len(), count);
    assert!(first.warnings.is_empty());
    assert!(Arc::ptr_eq(&first, &loader.get()));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[test]
fn embedded_backup_is_cached() {
    let mut loader = BundledCatalog::default();
    assert_eq!(
        (loader.asset)().unwrap(),
        fs::read("fixtures/model_prices_and_context_window_backup.json").unwrap()
    );
    assert_eq!(loader.get().entries.len(), 4458);
    let first = loader.get();
    assert!(Arc::ptr_eq(&first, &loader.get()));
}

#[test]
fn replays_the_model_catalog_corpus() {
    fs::read_dir(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fuzz/corpus/model_catalog"
    ))
    .unwrap()
    .map(|path| fs::read(path.unwrap().path()).unwrap())
    .for_each(|bytes| drop(ModelCatalog::parse(&bytes, UnknownFields::Warn)));
}

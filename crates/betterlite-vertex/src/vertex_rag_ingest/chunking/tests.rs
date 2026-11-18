#![cfg(test)]

use rstest::rstest;

use crate::vertex_rag_ingest::chunking::{
    ChunkDefaults, RagChunkingStrategy, SEPARATORS_IGNORED_WARNING, transform_chunking_strategy,
};
use crate::vertex_rag_ingest::tests::logs::install;

fn strategy(text: &str) -> RagChunkingStrategy {
    serde_json::from_str(text).expect("the strategy is valid")
}

#[rstest]
#[case::absent(None)]
#[case::empty_object(Some("{}"))]
#[case::auto_type(Some(r#"{"type":"auto"}"#))]
fn absent_or_empty_strategies_take_the_constants(#[case] text: Option<&str>) {
    let (_guard, logs) = install();
    let parsed = text.map(strategy);
    let record = transform_chunking_strategy(parsed.as_ref(), ChunkDefaults::LITELLM);
    assert_eq!(record.chunking_config.chunk_size, 1000);
    assert_eq!(record.chunking_config.chunk_overlap, 200);
    assert!(logs.warnings().is_empty());
}

#[rstest]
#[case::both_zero(r#"{"chunk_size":0,"chunk_overlap":0}"#, 0, 0)]
#[case::size_only(r#"{"chunk_size":512}"#, 512, 200)]
#[case::overlap_only(r#"{"chunk_overlap":32}"#, 1000, 32)]
fn explicit_values_keep_zero_and_default_only_the_missing_key(
    #[case] text: &str,
    #[case] chunk_size: i64,
    #[case] chunk_overlap: i64,
) {
    let strategy = strategy(text);
    let record = transform_chunking_strategy(Some(&strategy), ChunkDefaults::LITELLM);
    assert_eq!(record.chunking_config.chunk_size, chunk_size);
    assert_eq!(record.chunking_config.chunk_overlap, chunk_overlap);
}

#[rstest]
#[case::absent_strategy(None, 4096, 300)]
#[case::partial_strategy(Some(r#"{"chunk_size":512}"#), 512, 300)]
fn the_injected_defaults_fill_the_missing_keys(
    #[case] text: Option<&str>,
    #[case] chunk_size: i64,
    #[case] chunk_overlap: i64,
) {
    let parsed = text.map(strategy);
    let defaults = ChunkDefaults {
        chunk_size: 4096,
        chunk_overlap: 300,
    };
    let record = transform_chunking_strategy(parsed.as_ref(), defaults);
    assert_eq!(record.chunking_config.chunk_size, chunk_size);
    assert_eq!(record.chunking_config.chunk_overlap, chunk_overlap);
}

#[test]
fn custom_separators_warn_and_are_ignored() {
    let (_guard, logs) = install();
    let strategy = strategy(r#"{"chunk_size":10,"chunk_overlap":2,"separators":["\n\n",""]}"#);
    let record = transform_chunking_strategy(Some(&strategy), ChunkDefaults::LITELLM);
    assert_eq!(record.chunking_config.chunk_size, 10);
    assert_eq!(record.chunking_config.chunk_overlap, 2);
    assert_eq!(logs.warnings(), vec![SEPARATORS_IGNORED_WARNING.to_owned()]);
}

#[rstest]
#[case::empty(r#"{"separators":[]}"#)]
#[case::null(r#"{"separators":null}"#)]
fn empty_or_null_separators_do_not_warn(#[case] text: &str) {
    let (_guard, logs) = install();
    let strategy = strategy(text);
    let record = transform_chunking_strategy(Some(&strategy), ChunkDefaults::LITELLM);
    assert_eq!(record.chunking_config.chunk_size, 1000);
    assert!(logs.warnings().is_empty());
}

#[test]
fn an_unknown_strategy_key_is_refused() {
    let error = serde_json::from_str::<RagChunkingStrategy>(r#"{"chunker":"words"}"#)
        .expect_err("the unknown field is refused");
    assert!(error.to_string().contains("unknown field `chunker`"));
}

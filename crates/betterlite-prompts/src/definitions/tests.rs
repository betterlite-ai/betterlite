#![cfg(test)]

use super::expand::unpack_defs;
use super::legacy::{DEFS_MAX_INLINED_BYTES, unpack_legacy_defs};
use super::sanitize::sanitize_for_anthropic;
use crate::arguments::json::Object;
use rstest::rstest;
use std::{fs, path::PathBuf};

#[rstest]
fn replays_the_schema_definitions_corpus(
    #[files("fuzz/corpus/schema_definitions/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    if bytes.len() > 0x0001_0000 {
        return;
    }
    let Ok(schema) = serde_json::from_slice::<Object>(&bytes) else {
        return;
    };
    drop(unpack_defs(
        schema.clone(),
        &Object::new(),
        Some(DEFS_MAX_INLINED_BYTES),
    ));
    drop(unpack_legacy_defs(schema.clone(), DEFS_MAX_INLINED_BYTES));
    drop(sanitize_for_anthropic(schema));
}

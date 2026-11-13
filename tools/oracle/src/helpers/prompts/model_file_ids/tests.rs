#![cfg(test)]

use crate::case::{HelperCase, helper::validate_helper};
use std::{fs, path::Path};

#[test]
#[rustfmt::skip]
fn helper_seeds_select_model_file_ids() { let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case"); let seeds: Vec<String> = fs::read_dir(corpus).unwrap().map(|entry| entry.unwrap().path()).filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("model_file_ids"))).map(|path| fs::read_to_string(path).unwrap()).collect(); assert!(!seeds.is_empty()); assert!(seeds.iter().all(|source| { let case: HelperCase = serde_saphyr::from_str(source).unwrap(); serde_saphyr::from_str(source).map_err(|error| error.to_string()).and_then(|parsed| validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())).is_ok() && case.call.operation == "model_file_ids" })); }

#![cfg(test)]

use crate::case::{HelperCase, helper::validate_helper};
use rstest::rstest;
use std::{fs, path::Path};

#[rstest]
#[case::search_query_id("search-query-id")]
#[case::file_item("file-item")]
#[case::credentials("credentials")]
#[case::endpoints("endpoints")]
#[case::bad_reply("bad-reply")]
fn helper_seeds_select_openai_vector_stores(#[case] seed: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/helper_case")
        .join(format!("openai_vector_stores-{seed}.yaml"));
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    validate_helper(Path::new("<helper>"), &case).unwrap();
    assert_eq!(case.call.operation, "openai_vector_stores");
}

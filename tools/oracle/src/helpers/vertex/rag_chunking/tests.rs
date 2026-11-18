#![cfg(test)]

use std::fs;
use std::iter::once;
use std::path::{Path, PathBuf};

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;

use super::Op;

const OPERATION: &str = "vertex_rag_chunking";

#[rstest]
fn replays_the_vertex_rag_chunking_cases(
    #[base_dir = "../../.."]
    #[files("cases/helpers/vertex-rag-chunking/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).expect("the case is readable text");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("the case parses");
    validate_helper(&path, &case).expect("the case is valid");
    assert_eq!(case.call.operation, OPERATION);
    let outcome = run_case(&[crate::GROUP], &case, false).expect("the case runs");
    assert!(
        matches!(outcome, HelperRun::Match),
        "{} does not match LiteLLM",
        path.display()
    );
}

#[rstest]
fn replays_the_vertex_rag_chunking_seeds(
    #[base_dir = "../../.."]
    #[files("fuzz/corpus/helper_case/vertex_rag_chunking*")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).expect("the seed is readable text");
    let seed: HelperCase = serde_saphyr::from_str(&source).expect("the seed parses");
    validate_helper(Path::new("<helper>"), &seed).expect("the seed is a valid helper case");
    once(&seed.call)
        .chain(&seed.calls)
        .map(|call| {
            assert_eq!(call.operation, OPERATION);
            Op::check(&call.input)
        })
        .collect::<Result<Vec<_>, _>>()
        .expect("every seed input decodes into the typed input");
}

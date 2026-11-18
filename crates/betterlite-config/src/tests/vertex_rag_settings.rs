use std::collections::BTreeMap;

use betterlite_env::EnvError;
use rstest::rstest;

use crate::sections::Sections;

const CHUNK_SIZE: &str = "DEFAULT_CHUNK_SIZE";
const CHUNK_OVERLAP: &str = "DEFAULT_CHUNK_OVERLAP";

fn load(environment: &[(&str, &str)]) -> Result<(i64, i64), EnvError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Sections::vertex_rag_chunk_defaults(&lookup)
}

#[rstest]
#[case::absent(&[], 1000)]
#[case::explicit(&[(CHUNK_SIZE, "4096")], 4096)]
#[case::zero(&[(CHUNK_SIZE, "0")], 0)]
#[case::empty(&[(CHUNK_SIZE, "")], 1000)]
fn loads_default_chunk_size(#[case] environment: &[(&str, &str)], #[case] expected: i64) {
    let (chunk_size, chunk_overlap) = load(environment).unwrap();
    assert_eq!(chunk_size, expected);
    assert_eq!(chunk_overlap, 200);
}

#[rstest]
#[case::absent(&[], 200)]
#[case::explicit(&[(CHUNK_OVERLAP, "64")], 64)]
#[case::zero(&[(CHUNK_OVERLAP, "0")], 0)]
#[case::empty(&[(CHUNK_OVERLAP, "")], 200)]
fn loads_default_chunk_overlap(#[case] environment: &[(&str, &str)], #[case] expected: i64) {
    let (chunk_size, chunk_overlap) = load(environment).unwrap();
    assert_eq!(chunk_overlap, expected);
    assert_eq!(chunk_size, 1000);
}

#[test]
fn an_invalid_chunk_default_names_the_variable() {
    let error = load(&[(CHUNK_OVERLAP, "many")]).unwrap_err();
    assert_eq!(
        error,
        EnvError::Number {
            name: CHUNK_OVERLAP.to_owned()
        }
    );
}

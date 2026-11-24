use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::super::query::decode_scoped_report_query;

#[rstest]
fn replays_the_scoped_report_query_corpus(
    #[files("fuzz/corpus/scoped_report_query/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus seed is readable");
    let decoded = decode_scoped_report_query(&bytes);
    let again = decode_scoped_report_query(&bytes);
    assert_eq!(decoded, again);
}

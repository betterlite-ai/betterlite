use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::spend::global_report::query::{GroupBy, decode_global_report_query};

#[rstest]
fn replays_the_global_report_query_corpus(
    #[files("fuzz/corpus/global_report_query/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus seed is readable");
    let decoded = decode_global_report_query(&bytes);
    let again = decode_global_report_query(&bytes);
    assert!(decoded == again);
    let carries_group_by = form_urlencoded::parse(&bytes).any(|(name, _)| name == "group_by");
    if let Ok(query) = decoded
        && !carries_group_by
    {
        assert_eq!(query.group_by, GroupBy::Team);
    }
}

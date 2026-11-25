use std::fs;
use std::path::PathBuf;
use std::str;

use rstest::rstest;

use crate::model_info_search::datetime::decode_model_info_datetime;

#[rstest]
fn replays_the_model_info_datetime_corpus(
    #[files("fuzz/corpus/model_info_datetime/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("the corpus seed is readable");
    let text = str::from_utf8(&bytes).expect("a datetime seed is UTF-8");
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .expect("the seed has a UTF-8 name");
    let decoded = decode_model_info_datetime(text);
    if stem.starts_with("valid") {
        assert!(decoded.is_some(), "{stem} should decode");
    }
    if stem.starts_with("invalid") {
        assert!(decoded.is_none(), "{stem} should not decode");
    }
}

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::budget_permission_rows::JsonData;
use crate::ptu::charges::decode::decode_ptu_model_info;

fn decodes_anything(bytes: &[u8]) -> bool {
    let as_value = serde_json::from_slice::<JsonData>(bytes)
        .ok()
        .is_some_and(|value| decode_ptu_model_info(&value).is_some());
    let as_text = std::str::from_utf8(bytes)
        .ok()
        .is_some_and(|text| decode_ptu_model_info(&JsonData::String(text.to_owned())).is_some());
    as_value || as_text
}

#[rstest]
fn replays_the_ptu_model_info_corpus(#[files("fuzz/corpus/ptu_model_info/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    assert_eq!(
        decodes_anything(&bytes),
        name.starts_with("valid-"),
        "{name}"
    );
}

#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::config_fields::http::decode_body;
use crate::config_fields::records::{ConfigFieldDelete, ConfigFieldUpdate};
use crate::config_fields::validation::validate_field;

#[rstest]
fn replays_the_config_field_body_corpus(#[files("fuzz/corpus/config_field_body/*")] path: PathBuf) {
    let body = fs::read(path).expect("a corpus seed");
    if let Ok(update) = decode_body::<ConfigFieldUpdate>(&body) {
        drop(validate_field(&update.field_name, &update.field_value));
    }
    drop(decode_body::<ConfigFieldDelete>(&body));
}

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::ui_settings_state::decode::decode_stored_ui_settings;

#[rstest]
fn replays_the_stored_ui_settings_corpus(
    #[files("fuzz/corpus/stored_ui_settings/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_stored_ui_settings(&bytes));
}

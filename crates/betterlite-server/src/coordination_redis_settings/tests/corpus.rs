use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::coordination_redis_settings::decode::{
    decode_coordination_settings, decode_persisted_settings,
};

// The fuzz target runs the same two boundaries over the same seeds.
#[rstest]
fn replays_the_coordination_settings_corpus(
    #[files("fuzz/corpus/coordination_settings/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("a corpus seed");
    drop(decode_coordination_settings(&bytes));
    drop(decode_persisted_settings(&bytes));
}

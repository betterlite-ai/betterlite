#![cfg(test)]

use std::fs::read;
use std::path::PathBuf;

use rstest::rstest;

use crate::model_usage::decode::decode_model_usage_fields;
use crate::window_queue::stamp::decode_window_stamp;

mod catalog;
mod decode;
mod flush;
mod postgres;
mod rollup;
mod support;

#[rstest]
fn replays_the_rollup_inputs_corpus(#[files("fuzz/corpus/rollup_inputs/*")] path: PathBuf) {
    replay_rollup_input(&read(path).expect("a corpus file"));
}

// the tagged rollup_inputs fuzz target hands the rest of the input to one of the two decoders
fn replay_rollup_input(bytes: &[u8]) {
    let Some((&tag, rest)) = bytes.split_first() else {
        return;
    };
    if (tag & 1) == 0 {
        drop(decode_window_stamp(rest));
    } else {
        let mut parts = rest.split(|byte| *byte == 0);
        let metadata = parts.next().unwrap_or_default();
        let request_tags = parts.next().unwrap_or_default();
        drop(decode_model_usage_fields(metadata, request_tags));
    }
}

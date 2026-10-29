#![cfg(test)]

use super::concatenated::{salvage_concatenated_arguments, split_concatenated_json};
use super::parse::parse_tool_call_arguments;
use rstest::rstest;
use std::{fs, path::PathBuf, str};

#[rstest]
fn replays_the_tool_arguments_corpus(#[files("fuzz/corpus/tool_arguments/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if bytes.len() > 0x0001_0000 {
        return;
    }
    let Ok(arguments) = str::from_utf8(&bytes) else {
        return;
    };
    drop(parse_tool_call_arguments(arguments, None, None));
    drop(split_concatenated_json(arguments));
    drop(salvage_concatenated_arguments(arguments));
}

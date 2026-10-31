#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use rstest::rstest;

use crate::sources::activity_availability::ActivityAvailability;
use crate::sources::agent_row::AgentRow;
use crate::sources::count_row::CountRow;
use crate::sources::execution_row::ExecutionRow;
use crate::sources::parse_execution::parse_execution;
use crate::sources::part_row::PartRow;
use crate::sources::row_decode::decode_rows;

#[rstest]
fn replays_the_lens_sources_corpus(#[files("fuzz/corpus/lens_sources/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("reads the corpus file");
    if let Ok(text) = from_utf8(&bytes) {
        drop(parse_execution(text));
    }
    drop(decode_rows::<ActivityAvailability>(&bytes));
    drop(decode_rows::<ExecutionRow>(&bytes));
    drop(decode_rows::<PartRow>(&bytes));
    drop(decode_rows::<CountRow>(&bytes));
    drop(decode_rows::<AgentRow>(&bytes));
}

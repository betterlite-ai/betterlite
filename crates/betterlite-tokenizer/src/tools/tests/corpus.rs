use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::tools::{ToolLimits, format_function_definitions, parse_tool_definitions};

#[rstest]
fn replays_the_token_tool_format_corpus(#[files("fuzz/corpus/token_tool_format/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(tools) = parse_tool_definitions(&bytes, ToolLimits::default()) {
        let _formatted = format_function_definitions(&tools);
    }
}

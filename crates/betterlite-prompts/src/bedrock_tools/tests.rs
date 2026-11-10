#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::bedrock_tools::capabilities::{BedrockToolCapabilities, CachePointSupport};
use crate::bedrock_tools::parse::tools_from_json_bytes;

mod bedrock_tools_behavior_tests_behavior;

#[rstest]
fn replays_the_bedrock_native_tool_corpus(
    #[files("fuzz/corpus/bedrock_native_tool/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    if bytes.len() > 0x0001_0000 {
        return;
    }
    let capabilities = BedrockToolCapabilities {
        cache_points: CachePointSupport::ExtendedTtl,
        supports_strict_tools: true,
    };
    drop(tools_from_json_bytes(&bytes, capabilities));
}

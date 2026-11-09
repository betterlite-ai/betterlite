#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_logging::datadog_llm_obs::arguments::{ToolArguments, decode_tool_arguments};
use betterlite_logging::datadog_llm_obs::literal::literal_json;
use rstest::rstest;

#[rstest]
fn replays_the_datadog_llm_argument_decoder_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/datadog_llm_argument_decoder");
    let mut seen = 0_usize;
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let bytes = fs::read(&path).unwrap();
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        seen = seen.saturating_add(1);
        let _literal = literal_json(text);
        let decoded = decode_tool_arguments(text);
        if name.starts_with("object")
            || name.starts_with("nested")
            || name.starts_with("whitespace")
            || name.starts_with("deep")
        {
            assert!(matches!(decoded, ToolArguments::Object(_)), "{name}");
        } else {
            assert!(matches!(decoded, ToolArguments::Text(_)), "{name}");
        }
    }
    assert!(seen > 0, "the datadog llm argument decoder corpus is empty");
}

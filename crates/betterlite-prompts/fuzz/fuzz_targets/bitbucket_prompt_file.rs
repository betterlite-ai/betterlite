#![no_main]

use betterlite_prompts::bitbucket_prompt_manager::parse::{YamlParser, parse_prompt_file};
use betterlite_prompts::bitbucket_prompt_manager::yaml_basic::parse_yaml_basic;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 0x0001_0000 {
        return;
    }
    let Ok(content) = str::from_utf8(bytes) else {
        return;
    };
    drop(parse_prompt_file(content, "fuzz", YamlParser::Full));
    drop(parse_prompt_file(content, "fuzz", YamlParser::Basic));
    drop(parse_yaml_basic(content));
});

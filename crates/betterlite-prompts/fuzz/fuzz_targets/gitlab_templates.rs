#![no_main]

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::gitlab_client::payload::{decode_file_payload, decode_text};
use betterlite_prompts::gitlab_templates::parse::{
    FrontmatterParser, parse_prompt_file, parse_yaml_basic,
};
use betterlite_prompts::gitlab_templates::sandbox::render_sandboxed;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 0x0001_0000 {
        return;
    }
    drop(decode_file_payload(bytes));
    drop(decode_text(bytes));
    let Ok(content) = str::from_utf8(bytes) else {
        return;
    };
    drop(parse_yaml_basic(content));
    for parser in [FrontmatterParser::Yaml, FrontmatterParser::Basic] {
        if let Ok(template) = parse_prompt_file(content, "fuzz", parser) {
            drop(render_sandboxed("fuzz", &template.content, &Object::new()));
        }
    }
});

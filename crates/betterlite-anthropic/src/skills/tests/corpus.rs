use std::fs;
use std::path::Path;

use super::support::{DEFAULT_BASE, anthropic_config};
use crate::skills::config::{AnthropicSkillsConfig, BaseSkillsApiConfig as _};

fn expected(config: &AnthropicSkillsConfig, name: &str, bytes: &[u8]) -> bool {
    if name.starts_with("skill_valid") {
        config.transform_create_skill_response(bytes).is_ok()
    } else if name.starts_with("skill_invalid") {
        config.transform_create_skill_response(bytes).is_err()
    } else if name.starts_with("list_valid") {
        config.transform_list_skills_response(bytes).is_ok()
    } else if name.starts_with("list_invalid") {
        config.transform_list_skills_response(bytes).is_err()
    } else if name.starts_with("delete_valid") {
        config.transform_delete_skill_response(bytes).is_ok()
    } else if name.starts_with("delete_invalid") {
        config.transform_delete_skill_response(bytes).is_err()
    } else {
        false
    }
}

#[test]
fn replays_the_skills_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_skills");
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    let files: Vec<(String, Vec<u8>)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            (name, fs::read(path).unwrap())
        })
        .collect();
    assert!(files.len() >= 6);
    let outcomes: Vec<(String, bool)> = files
        .iter()
        .map(|(name, bytes)| (name.clone(), expected(&config, name, bytes)))
        .collect();
    assert!(
        outcomes.iter().all(|(_, accepted)| *accepted),
        "{outcomes:?}"
    );
}

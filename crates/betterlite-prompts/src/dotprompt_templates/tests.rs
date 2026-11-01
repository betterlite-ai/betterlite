#![cfg(test)]

use super::PromptManager;
use super::content::dotprompt_content_to_json;
use rstest::rstest;
use std::{fs, path::PathBuf, str};

#[rstest]
fn replays_the_dotprompt_templates_corpus(
    #[files("fuzz/corpus/dotprompt_templates/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    if bytes.len() > 0x0001_0000 {
        return;
    }
    let Ok(content) = str::from_utf8(&bytes) else {
        return;
    };

    drop(dotprompt_content_to_json(content));

    let mut manager = PromptManager::new(None, None, None, None).unwrap();
    if manager.add_prompt("fuzz", content.to_owned(), None).is_ok() {
        drop(manager.render("fuzz", None, None));
    }

    if let Ok(prompt_data) = serde_json::from_str(content) {
        manager.load_prompts_from_json_data(&prompt_data);
    }
}

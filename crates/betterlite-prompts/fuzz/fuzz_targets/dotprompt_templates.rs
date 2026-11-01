#![no_main]

use betterlite_prompts::dotprompt_templates::PromptManager;
use betterlite_prompts::dotprompt_templates::content::dotprompt_content_to_json;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 65_536 {
        return;
    }
    let Ok(content) = std::str::from_utf8(bytes) else {
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
});

#![no_main]

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::dotprompt_templates::PromptManager;
use betterlite_prompts::dotprompt_templates::content::dotprompt_content_to_json;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 0x0001_0000 {
        return;
    }
    let Ok(content) = str::from_utf8(bytes) else {
        return;
    };

    drop(dotprompt_content_to_json(content));

    let Ok(mut manager) = PromptManager::new(None, None, None, None) else {
        return;
    };
    if manager
        .add_prompt("fuzz", content.to_owned(), Object::new())
        .is_ok()
    {
        drop(manager.render("fuzz", &Object::new(), None));
    }

    if let Ok(prompt_data) = serde_json::from_str(content) {
        manager.load_prompts_from_json_data(&prompt_data);
    }
});

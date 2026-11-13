#![no_main]
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::skills::config::{AnthropicSkillsConfig, BaseSkillsApiConfig as _};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let config = AnthropicSkillsConfig {
        settings: AnthropicSettings::default(),
    };
    let _skill = config.transform_get_skill_response(input);
    let _list = config.transform_list_skills_response(input);
    // ast-grep-ignore: no-long-names
    let _deleted = config.transform_delete_skill_response(input);
});

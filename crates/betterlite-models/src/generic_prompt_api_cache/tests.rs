#![cfg(test)]

use rstest::rstest;

use crate::decisions::Json;
use crate::generic_prompt_api::test_support::{system_content, template_manager, variables};
use crate::generic_prompt_api_cache::PromptCache;
use crate::prompt_contracts::PromptCompileRequest;

#[rstest]
#[case::all_present(Some("welcome"), Some("prod"), Some(3), "welcome:prod:3")]
#[case::none_values_are_spelled_out(Some("welcome"), None, None, "welcome:None:None")]
#[case::no_prompt_id(None, Some("prod"), None, "None:prod:None")]
fn cache_key_joins_id_label_and_version_with_colons(
    #[case] prompt_id: Option<&str>,
    #[case] label: Option<&str>,
    #[case] version: Option<i64>,
    #[case] expected: &str,
) {
    assert_eq!(
        PromptCache::get_cache_key(prompt_id, label, version),
        expected
    );
}

#[test]
fn common_caching_logic_returns_none_when_nothing_is_cached() {
    let (manager, requests) = template_manager();
    let found = manager
        .prompt_cache
        .common_caching_logic(Some("welcome"), None, None, None)
        .unwrap();
    assert_eq!(found, None);
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn a_cache_hit_without_variables_serves_a_later_request_with_variables() {
    let (manager, requests) = template_manager();
    let dynamic = Json::Null;
    let plain = PromptCompileRequest {
        prompt_id: Some("welcome"),
        prompt_spec: None,
        prompt_variables: None,
        dynamic_callback_params: &dynamic,
        prompt_label: None,
        prompt_version: None,
    };
    let raw = manager.compile_prompt_helper(&plain).unwrap();
    assert_eq!(
        system_content(&raw.prompt_template),
        Some("Hello {name}, welcome to {{place}}. {name} again.")
    );

    let values = variables(&[("name", Json::Text("Ada".to_owned()))]);
    let filled = manager
        .prompt_cache
        .common_caching_logic(Some("welcome"), None, None, Some(&values))
        .unwrap()
        .unwrap();
    assert_eq!(
        system_content(&filled.prompt_template),
        Some("Hello Ada, welcome to {{place}}. Ada again.")
    );

    let again = manager.compile_prompt_helper(&plain).unwrap();
    assert_eq!(again, raw);
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[test]
fn an_empty_variable_map_returns_the_cached_template_untouched() {
    let (manager, _) = template_manager();
    let dynamic = Json::Null;
    let plain = PromptCompileRequest {
        prompt_id: Some("welcome"),
        prompt_spec: None,
        prompt_variables: None,
        dynamic_callback_params: &dynamic,
        prompt_label: None,
        prompt_version: None,
    };
    let raw = manager.compile_prompt_helper(&plain).unwrap();
    let empty = variables(&[]);
    let found = manager
        .prompt_cache
        .common_caching_logic(Some("welcome"), None, None, Some(&empty))
        .unwrap();
    assert_eq!(found, Some(raw));
}

#[test]
fn label_and_version_each_get_their_own_cache_entry() {
    let (manager, requests) = template_manager();
    let dynamic = Json::Null;
    let base = PromptCompileRequest {
        prompt_id: Some("welcome"),
        prompt_spec: None,
        prompt_variables: None,
        dynamic_callback_params: &dynamic,
        prompt_label: None,
        prompt_version: None,
    };
    let labeled = PromptCompileRequest {
        prompt_label: Some("prod"),
        ..base
    };
    manager.compile_prompt_helper(&labeled).unwrap();
    let versioned = PromptCompileRequest {
        prompt_id: Some("welcome"),
        prompt_spec: None,
        prompt_variables: None,
        dynamic_callback_params: &dynamic,
        prompt_label: Some("prod"),
        prompt_version: Some(2),
    };
    manager.compile_prompt_helper(&versioned).unwrap();
    manager.compile_prompt_helper(&labeled).unwrap();
    assert_eq!(requests.lock().unwrap().len(), 2);
    let keys: Vec<String> = {
        let mut keys: Vec<String> = manager
            .prompt_cache
            .entries
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    };
    assert_eq!(keys, vec!["welcome:prod:2", "welcome:prod:None"]);
}

#[test]
fn clear_cache_makes_the_next_compile_fetch_again() {
    let (manager, requests) = template_manager();
    let dynamic = Json::Null;
    let request = PromptCompileRequest {
        prompt_id: Some("welcome"),
        prompt_spec: None,
        prompt_variables: None,
        dynamic_callback_params: &dynamic,
        prompt_label: None,
        prompt_version: None,
    };
    manager.compile_prompt_helper(&request).unwrap();
    manager.compile_prompt_helper(&request).unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);

    manager.prompt_cache.clear_cache();
    assert!(manager.prompt_cache.entries.lock().unwrap().is_empty());
    manager.compile_prompt_helper(&request).unwrap();
    assert_eq!(requests.lock().unwrap().len(), 2);
}

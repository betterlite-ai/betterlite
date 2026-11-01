#![cfg(test)]

use super::{GlobalPromptDirectory, set_global_prompt_directory};

#[test]
fn fresh_handle_has_no_directory() {
    let global = GlobalPromptDirectory::default();

    assert_eq!(global.get(), None);
}

#[test]
fn set_then_get_returns_the_directory() {
    let global = GlobalPromptDirectory::default();

    set_global_prompt_directory(&global, "/prompts".to_owned());

    assert_eq!(global.get(), Some("/prompts".to_owned()));
}

#[test]
fn clones_of_a_handle_share_the_same_directory() {
    let global = GlobalPromptDirectory::default();
    let shared = global.clone();

    set_global_prompt_directory(&shared, "/other".to_owned());

    assert_eq!(global.get(), Some("/other".to_owned()));
}

#[test]
fn setting_again_overwrites_the_previous_directory() {
    let global = GlobalPromptDirectory::default();
    set_global_prompt_directory(&global, "/first".to_owned());

    set_global_prompt_directory(&global, "/second".to_owned());

    assert_eq!(global.get(), Some("/second".to_owned()));
}

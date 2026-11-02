#![cfg(test)]

use std::fs;
use std::sync::PoisonError;

use super::GlobalPromptDirectory;
use crate::dotprompt_compilation::init::manager::DotpromptManager;

#[test]
fn a_directory_set_through_a_cloned_handle_is_used_by_a_manager_built_from_the_original() {
    let directory = tempfile::tempdir().expect("creates a temp directory");
    fs::write(directory.path().join("greeting.prompt"), "User: hi").expect("writes the seed file");

    let global = GlobalPromptDirectory::default();
    let shared = global.clone();
    *shared.0.lock().unwrap_or_else(PoisonError::into_inner) =
        Some(directory.path().display().to_string());

    let manager =
        DotpromptManager::new(None, None, None, None, &global).expect("no prompt_data to decode");

    assert!(manager.should_run_prompt_management(Some("greeting")));
}

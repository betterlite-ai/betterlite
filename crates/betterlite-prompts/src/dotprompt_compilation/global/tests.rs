#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};

use super::GlobalPromptDirectory;

#[test]
fn fresh_handle_has_no_directory() {
    let global = GlobalPromptDirectory::default();

    assert_eq!(
        global
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        None
    );
}

#[test]
fn set_then_get_returns_the_directory() {
    let global = GlobalPromptDirectory(Arc::new(Mutex::new(Some("/prompts".to_owned()))));

    assert_eq!(
        global
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        Some("/prompts".to_owned())
    );
}

#[test]
fn clones_of_a_handle_share_the_same_directory() {
    let global = GlobalPromptDirectory::default();
    let shared = global.clone();

    *shared.0.lock().unwrap_or_else(PoisonError::into_inner) = Some("/other".to_owned());

    assert_eq!(
        global
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        Some("/other".to_owned())
    );
}

#[test]
fn setting_again_overwrites_the_previous_directory() {
    let global = GlobalPromptDirectory(Arc::new(Mutex::new(Some("/first".to_owned()))));

    *global.0.lock().unwrap_or_else(PoisonError::into_inner) = Some("/second".to_owned());

    assert_eq!(
        global
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        Some("/second".to_owned())
    );
}

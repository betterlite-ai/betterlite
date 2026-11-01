use std::sync::{Arc, Mutex, PoisonError};

#[derive(Clone, Debug, Default)]
pub struct GlobalPromptDirectory(Arc<Mutex<Option<String>>>);

impl GlobalPromptDirectory {
    #[must_use]
    pub fn get(&self) -> Option<String> {
        let guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }
}

pub fn set_global_prompt_directory(global: &GlobalPromptDirectory, directory: String) {
    let mut guard = global.0.lock().unwrap_or_else(PoisonError::into_inner);
    *guard = Some(directory);
}

mod tests;

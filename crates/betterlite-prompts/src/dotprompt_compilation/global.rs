use std::sync::{Arc, Mutex};

// No get/set methods here: callers lock the cell directly instead of a one-line wrapper over it.
#[derive(Clone, Debug, Default)]
pub struct GlobalPromptDirectory(pub(crate) Arc<Mutex<Option<String>>>);

mod tests;

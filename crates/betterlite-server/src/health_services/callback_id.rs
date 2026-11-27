use crate::health_readiness::callback_listing::{CallbackEntry, callback_name};

// LiteLLM checks a callback_name attribute and the class registry first; B189's registry already holds one name per callback
#[must_use]
pub fn get_callback_identifier(callback: &CallbackEntry) -> String {
    match callback {
        CallbackEntry::Plain(name) => name.clone(),
        CallbackEntry::Registered(_) => callback_name(callback),
    }
}

use super::DynamicCallback;

#[must_use]
pub fn get_combined_callback_list(
    dynamic: Option<&[DynamicCallback]>,
    global: &[DynamicCallback],
) -> Vec<DynamicCallback> {
    let Some(selected) = dynamic else {
        return global.to_vec();
    };
    let mut combined = Vec::new();
    for callback in selected.iter().chain(global) {
        if !combined.contains(callback) {
            combined.push(callback.clone());
        }
    }
    combined
}

#[must_use]
pub fn get_callback_name(callback: &DynamicCallback) -> &str {
    match callback {
        DynamicCallback::Name(name) => name,
        DynamicCallback::Logger(logger) => logger.callback_name(),
    }
}

// LiteLLM only exempts plain callables; a logger is not callable, so it falls into the internal branch
#[must_use]
pub const fn is_internal_callback(callback: &DynamicCallback) -> bool {
    match callback {
        DynamicCallback::Name(_) => false,
        DynamicCallback::Logger(_) => true,
    }
}

#[must_use]
pub fn remove_internal_callbacks(callbacks: &[DynamicCallback]) -> Vec<DynamicCallback> {
    callbacks
        .iter()
        .filter(|callback| !is_internal_callback(callback))
        .cloned()
        .collect()
}

// LiteLLM drops logger instances and names in its known-callback list, and keeps every other string
#[must_use]
pub fn remove_logger_callbacks(
    callbacks: &[DynamicCallback],
    is_known: &dyn Fn(&str) -> bool,
) -> Vec<DynamicCallback> {
    callbacks
        .iter()
        .filter(|callback| match callback {
            DynamicCallback::Name(name) => !is_known(name),
            DynamicCallback::Logger(_) => false,
        })
        .cloned()
        .collect()
}

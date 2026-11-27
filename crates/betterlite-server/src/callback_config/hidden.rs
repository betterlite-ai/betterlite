use std::collections::BTreeSet;

use super::ports::{RuntimeCallback, RuntimeCallbacks};

// _hidden_runtime_callback_names always hides these two, before it looks at any object
const INTERNAL_CALLBACK_NAMES: [&str; 2] = ["cache", "vector_store_pre_call_hook"];
const INTERNAL_MODULE_OWNERS: [&str; 2] = ["litellm", "litellm_enterprise"];

// The runtime callbacks the settings page must not offer: LiteLLM's own machinery, guardrails and configured objects
pub(crate) fn hidden_names(
    callbacks: &dyn RuntimeCallbacks,
    configured: &BTreeSet<String>,
) -> BTreeSet<String> {
    let classes: BTreeSet<String> = configured
        .iter()
        .filter_map(|name| callbacks.class_str_for_name(name))
        .collect();
    let modules: BTreeSet<String> = configured
        .iter()
        .filter_map(|name| {
            name.rsplit_once('.')
                .map(|(module, _rest)| module.to_owned())
        })
        .collect();
    let mut hidden: BTreeSet<String> = INTERNAL_CALLBACK_NAMES
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    for callback in callbacks.objects() {
        if is_hidden(&callback, &classes, &modules) {
            hidden.insert(callback.name.clone());
        }
    }
    hidden
}

fn is_hidden(
    callback: &RuntimeCallback,
    configured_classes: &BTreeSet<String>,
    configured_modules: &BTreeSet<String>,
) -> bool {
    callback.guardrail
        || is_internal(callback)
        || is_configured_class_instance(callback, configured_classes)
        || configured_modules.contains(&callback.module)
}

fn is_internal(callback: &RuntimeCallback) -> bool {
    let owner = callback.module.split('.').next().unwrap_or_default();
    !callback.registered_integration && INTERNAL_MODULE_OWNERS.contains(&owner)
}

// A self-naming instance matches by its own name, so a configured bare `logfire` leaves its siblings of the class visible
fn is_configured_class_instance(
    callback: &RuntimeCallback,
    configured_classes: &BTreeSet<String>,
) -> bool {
    let Some(class_str) = callback.class_str.as_ref() else {
        return false;
    };
    configured_classes.contains(class_str)
        && (callback.name == *class_str || callback.name == callback.type_name)
}

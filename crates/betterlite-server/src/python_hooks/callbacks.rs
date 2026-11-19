use betterlite_pyhost::hook_object::LoadedKind;
use pyo3::types::PyAnyMethods as _;

use super::HooksError;
use super::slots::LoadedHook;

// B6030 ships the class at this path; until then nothing matches it and the callable check decides
const LOGGER_CLASS: &str = "betterlite.integrations.custom_logger.CustomLogger";

pub(super) fn check_dispatchable(hook: &LoadedHook, entry: &str) -> Result<(), HooksError> {
    match hook.object.kind(&hook.host, LOGGER_CLASS) {
        LoadedKind::Logger | LoadedKind::Function => Ok(()),
        LoadedKind::Class { module, qualname } => Err(HooksError::CallbackIsClass {
            entry: entry.to_owned(),
            module,
            qualname,
            name: class_name(hook),
            module_path: module_path(entry),
        }),
        LoadedKind::Other { type_name, repr } => Err(HooksError::CallbackNotDispatchable {
            entry: entry.to_owned(),
            type_name,
            repr,
        }),
    }
}

fn class_name(hook: &LoadedHook) -> String {
    hook.host.attach(|py| {
        hook.object
            .loaded
            .object
            .bind(py)
            .getattr("__name__")
            .ok()
            .and_then(|value| value.extract::<String>().ok())
            .unwrap_or_default()
    })
}

fn module_path(entry: &str) -> String {
    entry
        .rsplit_once('.')
        .map_or_else(|| entry.to_owned(), |(path, _class)| path.to_owned())
}

mod tests;

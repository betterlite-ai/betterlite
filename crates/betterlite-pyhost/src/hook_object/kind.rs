use pyo3::types::{PyAnyMethods as _, PyType};
use pyo3::{Bound, PyAny, Python};

use crate::json_values::object_type_name;

use super::{LoadedKind, class_object};

pub(crate) fn classify(
    py: Python<'_>,
    object: &Bound<'_, PyAny>,
    logger_class: &str,
) -> LoadedKind {
    if class_object(py, logger_class).is_ok_and(|class| object.is_instance(&class).unwrap_or(false))
    {
        return LoadedKind::Logger;
    }
    if object.is_instance_of::<PyType>() {
        return LoadedKind::Class {
            module: text_attribute(object, "__module__"),
            qualname: text_attribute(object, "__qualname__"),
        };
    }
    if object.is_callable() {
        return LoadedKind::Function;
    }
    LoadedKind::Other {
        type_name: object_type_name(object),
        repr: repr_text(object),
    }
}

fn text_attribute(object: &Bound<'_, PyAny>, name: &str) -> String {
    object
        .getattr_opt(name)
        .ok()
        .flatten()
        .and_then(|value| value.extract::<String>().ok())
        .unwrap_or_default()
}

fn repr_text(object: &Bound<'_, PyAny>) -> String {
    object
        .repr()
        .and_then(|text| text.extract::<String>())
        .unwrap_or_else(|_error| object_type_name(object))
}

mod tests;

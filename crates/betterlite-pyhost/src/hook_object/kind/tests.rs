#![cfg(test)]

use std::ffi::CStr;

use pyo3::types::{PyAnyMethods as _, PyModule, PyString};
use pyo3::{Py, PyAny};

use crate::hook_object::{HookObject, LoadedKind};
use crate::host::{HostConfig, PythonHost};
use crate::instance_loader::LoadedObject;

const LOGGER_CODE: &CStr = c"class CustomLogger:\n    def log(self):\n        return 'logged'\n\nclass CallableLogger(CustomLogger):\n    def __call__(self):\n        return 'called'\n";

const FUNCTION_CODE: &CStr = c"def hook():\n    return 1\n";

const CLASS_CODE: &CStr = c"class Handler:\n    pass\n";

const HOLDER_CODE: &CStr = c"class Holder:\n    pass\n";

const PARTIAL_CODE: &CStr = c"import functools\n\nhandler = functools.partial(len)\n";

fn start_host() -> PythonHost {
    PythonHost::start(&HostConfig::default()).unwrap()
}

fn registered_module(host: &PythonHost, name: &CStr, code: &CStr) -> Py<PyAny> {
    host.attach(|py| {
        let module = PyModule::from_code(py, code, c"b8043_kind.py", name).unwrap();
        py.import("sys")
            .unwrap()
            .getattr("modules")
            .unwrap()
            .set_item(name.to_str().unwrap(), &module)
            .unwrap();
        module.into_any().unbind()
    })
}

fn attribute(host: &PythonHost, module: &Py<PyAny>, name: &str) -> Py<PyAny> {
    host.attach(|py| module.bind(py).getattr(name).unwrap().unbind())
}

fn instance_of(host: &PythonHost, module_name: &str, class_name: &str) -> Py<PyAny> {
    let class = host.attach(|py| {
        py.import("importlib")
            .unwrap()
            .call_method1("import_module", (module_name,))
            .unwrap()
            .getattr(class_name)
            .unwrap()
            .unbind()
    });
    host.attach(|py| class.bind(py).call0().unwrap().unbind())
}

fn hook_of(object: Py<PyAny>) -> HookObject {
    HookObject {
        loaded: LoadedObject {
            object,
            value: "b8043_kind.value".to_owned(),
        },
    }
}

#[test]
fn a_logger_instance_is_a_logger() {
    let host = start_host();
    registered_module(&host, c"b8043_logger", LOGGER_CODE);
    let listener = instance_of(&host, "b8043_logger", "CustomLogger");
    let kind = hook_of(listener).kind(&host, "b8043_logger.CustomLogger");
    assert_eq!(kind, LoadedKind::Logger);
}

#[test]
fn a_callable_logger_instance_is_a_logger() {
    let host = start_host();
    registered_module(&host, c"b8043_callable_logger", LOGGER_CODE);
    let listener = instance_of(&host, "b8043_callable_logger", "CallableLogger");
    let kind = hook_of(listener).kind(&host, "b8043_callable_logger.CustomLogger");
    assert_eq!(kind, LoadedKind::Logger);
}

#[test]
fn a_plain_function_is_a_function() {
    let host = start_host();
    let module = registered_module(&host, c"b8043_function", FUNCTION_CODE);
    let hook = attribute(&host, &module, "hook");
    let kind = hook_of(hook).kind(&host, "b8043_function.CustomLogger");
    assert_eq!(kind, LoadedKind::Function);
}

#[test]
fn a_partial_is_a_function() {
    let host = start_host();
    let module = registered_module(&host, c"b8043_partial", PARTIAL_CODE);
    let handler = attribute(&host, &module, "handler");
    let kind = hook_of(handler).kind(&host, "b8043_partial.CustomLogger");
    assert_eq!(kind, LoadedKind::Function);
}

#[test]
fn a_class_is_a_class_with_its_module_and_qualified_name() {
    let host = start_host();
    let module = registered_module(&host, c"b8043_class", CLASS_CODE);
    let handler = attribute(&host, &module, "Handler");
    let kind = hook_of(handler).kind(&host, "b8043_class.Response");
    assert_eq!(
        kind,
        LoadedKind::Class {
            module: "b8043_class".to_owned(),
            qualname: "Handler".to_owned(),
        }
    );
}

#[test]
fn a_class_is_not_a_function_even_though_it_is_callable() {
    let host = start_host();
    let module = registered_module(&host, c"b8043_callable_class", CLASS_CODE);
    let handler = attribute(&host, &module, "Handler");
    let kind = hook_of(handler).kind(&host, "b8043_callable_class.Response");
    assert!(matches!(kind, LoadedKind::Class { .. }), "got: {kind:?}");
}

#[test]
fn a_string_is_other_with_its_type_name_and_repr() {
    let host = start_host();
    let text = host.attach(|py| PyString::new(py, "text").into_any().unbind());
    let kind = hook_of(text).kind(&host, "b8043_logger.CustomLogger");
    assert_eq!(
        kind,
        LoadedKind::Other {
            type_name: "str".to_owned(),
            repr: "'text'".to_owned(),
        }
    );
}

#[test]
fn a_plain_instance_is_other_with_its_type_name_and_repr() {
    let host = start_host();
    registered_module(&host, c"b8043_holder", HOLDER_CODE);
    let holder = instance_of(&host, "b8043_holder", "Holder");
    let kind = hook_of(holder).kind(&host, "b8043_holder.CustomLogger");
    let LoadedKind::Other { type_name, repr } = kind else {
        panic!("expected Other, got {kind:?}");
    };
    assert_eq!(type_name, "Holder");
    assert!(
        repr.starts_with("<b8043_holder.Holder object at "),
        "got: {repr}"
    );
}

#[test]
fn an_unimportable_logger_class_means_not_that_class() {
    let host = start_host();
    let module = registered_module(&host, c"b8043_no_logger", FUNCTION_CODE);
    let hook = attribute(&host, &module, "hook");
    let kind = hook_of(hook).kind(&host, "b8043_absent_module.CustomLogger");
    assert_eq!(kind, LoadedKind::Function);
}

#[test]
fn a_logger_class_without_a_module_part_means_not_that_class() {
    let host = start_host();
    let module = registered_module(&host, c"b8043_bare_logger", FUNCTION_CODE);
    let hook = attribute(&host, &module, "hook");
    let kind = hook_of(hook).kind(&host, "CustomLogger");
    assert_eq!(kind, LoadedKind::Function);
}

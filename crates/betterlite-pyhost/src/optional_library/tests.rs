#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _, PyModule};

use super::{BlockingCall, LibraryError, OptionalLibrary};
use crate::blocking_work::WorkTracker;
use crate::call_error::CallError;
use crate::host::{DEFAULT_SHUTDOWN_GRACE, HostConfig, PythonHost};
use crate::json_values::{JsonObject, JsonValue};

fn start_host(module_dirs: Vec<PathBuf>) -> PythonHost {
    PythonHost::start(&HostConfig {
        module_dirs,
        shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
    })
    .unwrap()
}

fn host_with_files(files: &[(&str, &str)]) -> (PythonHost, tempfile::TempDir) {
    let folder = tempfile::tempdir().unwrap();
    for (name, code) in files {
        fs::write(folder.path().join(name), code).unwrap();
    }
    (start_host(vec![folder.path().to_path_buf()]), folder)
}

fn tracker() -> WorkTracker {
    WorkTracker::new(2, Duration::from_secs(30))
}

fn object_value(value: &JsonValue, name: &str) -> JsonValue {
    let JsonValue::Object(entries) = value else {
        panic!("expected an object, got {value:?}");
    };
    entries.get(name).cloned().unwrap()
}

fn register_late_library(host: &PythonHost) {
    host.attach(|py| {
        let module = PyModule::from_code(
            py,
            c"def value():\n    return 7\n",
            c"b8043_late_library.py",
            c"b8043_late_library",
        )
        .unwrap();
        py.import("sys")
            .unwrap()
            .getattr("modules")
            .unwrap()
            .set_item("b8043_late_library", module)
            .unwrap();
    });
}

#[test]
fn a_missing_library_reports_the_install_hint() {
    let host = start_host(Vec::new());
    let library = OptionalLibrary::default();
    let error = library
        .import(
            &host,
            "b8043_no_such_library",
            "pip install b8043-no-such-library",
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "b8043_no_such_library is not installed. Install it with: pip install b8043-no-such-library"
    );
    assert!(matches!(
        error,
        LibraryError::NotInstalled { module, .. } if module == "b8043_no_such_library"
    ));
}

#[test]
fn an_import_error_inside_an_installed_library_is_not_missing() {
    let (host, _folder) = host_with_files(&[(
        "b8043_broken_library.py",
        "import b8043_absent_dependency\n",
    )]);
    let library = OptionalLibrary::default();
    let error = library
        .import(
            &host,
            "b8043_broken_library",
            "pip install b8043-broken-library",
        )
        .unwrap_err();
    let LibraryError::Import { source } = error else {
        panic!("expected an import failure, got {error:?}");
    };
    assert_eq!(source.type_name, "ModuleNotFoundError");
    assert!(
        source.message.contains("b8043_absent_dependency"),
        "got: {}",
        source.message
    );
}

#[test]
fn an_import_error_from_an_installed_package_is_not_missing() {
    let folder = tempfile::tempdir().unwrap();
    let package = folder.path().join("b8043_installed_package");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("__init__.py"), "from . import b8043_absent\n").unwrap();
    let host = start_host(vec![folder.path().to_path_buf()]);
    let library = OptionalLibrary::default();
    let error = library
        .import(
            &host,
            "b8043_installed_package",
            "pip install b8043-installed-package",
        )
        .unwrap_err();
    let LibraryError::Import { source } = error else {
        panic!("expected an import failure, got {error:?}");
    };
    assert_eq!(source.type_name, "ImportError");
    assert!(
        source.message.contains("b8043_absent"),
        "got: {}",
        source.message
    );
}

#[test]
fn a_missing_top_level_package_of_a_submodule_is_not_installed() {
    let host = start_host(Vec::new());
    let library = OptionalLibrary::default();
    let error = library
        .import(
            &host,
            "b8043_absent_package.sub",
            "pip install b8043-absent-package",
        )
        .unwrap_err();
    let LibraryError::NotInstalled {
        module,
        install_hint,
    } = error
    else {
        panic!("expected a missing library, got {error:?}");
    };
    assert_eq!(module, "b8043_absent_package.sub");
    assert_eq!(install_hint, "pip install b8043-absent-package");
}

#[tokio::test]
async fn a_failed_import_is_not_cached_once_the_library_appears() {
    let host = start_host(Vec::new());
    let library = OptionalLibrary::default();
    let error = library
        .import(
            &host,
            "b8043_late_library",
            "pip install b8043-late-library",
        )
        .unwrap_err();
    assert!(matches!(error, LibraryError::NotInstalled { .. }));
    register_late_library(&host);
    let module = library
        .import(
            &host,
            "b8043_late_library",
            "pip install b8043-late-library",
        )
        .unwrap();
    let function = module.function(&host, "value").unwrap();
    let value = function
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            Vec::new(),
            JsonObject::new(),
        )
        .await
        .unwrap();
    assert_eq!(value, JsonValue::from(7_i64));
}

#[test]
fn a_successful_import_is_cached_per_name() {
    let (host, _folder) = host_with_files(&[
        ("b8043_counter.py", "count = 0\n"),
        (
            "b8043_counted_library.py",
            "import b8043_counter\nb8043_counter.count += 1\n",
        ),
    ]);
    let library = OptionalLibrary::default();
    library
        .import(
            &host,
            "b8043_counted_library",
            "pip install b8043-counted-library",
        )
        .unwrap();
    host.attach(|py| {
        let modules = py.import("sys").unwrap().getattr("modules").unwrap();
        modules
            .cast_into::<PyDict>()
            .unwrap()
            .del_item("b8043_counted_library")
            .unwrap();
    });
    library
        .import(
            &host,
            "b8043_counted_library",
            "pip install b8043-counted-library",
        )
        .unwrap();
    let count = host.attach(|py| {
        py.import("b8043_counter")
            .unwrap()
            .getattr("count")
            .unwrap()
            .extract::<i64>()
            .unwrap()
    });
    assert_eq!(count, 1, "the second import ran the library code again");
}

#[tokio::test]
async fn function_walks_attributes_to_a_standard_library_function() {
    let host = start_host(Vec::new());
    let library = OptionalLibrary::default();
    let os = library.import(&host, "os", "pip install os").unwrap();
    let join = os.function(&host, "path.join").unwrap();
    let value = join
        .call(
            &host,
            &tracker(),
            Duration::from_secs(5),
            vec![JsonValue::from("a"), JsonValue::from("b")],
            JsonObject::new(),
        )
        .await
        .unwrap();
    assert_eq!(value, JsonValue::from("a/b"));
}

#[test]
fn function_reports_a_missing_attribute() {
    let host = start_host(Vec::new());
    let library = OptionalLibrary::default();
    let os = library.import(&host, "os", "pip install os").unwrap();
    let error = os.function(&host, "path.b8043_missing").unwrap_err();
    let LibraryError::MissingAttribute {
        module,
        path,
        source,
    } = error
    else {
        panic!("expected a missing attribute, got {error:?}");
    };
    assert_eq!(module, "os");
    assert_eq!(path, "path.b8043_missing");
    assert_eq!(source.type_name, "AttributeError");
}

#[test]
fn function_reports_an_attribute_error_that_is_not_a_missing_name() {
    let (host, _folder) = host_with_files(&[(
        "b8043_strict_library.py",
        "def __getattr__(name):\n    raise RuntimeError('module attribute failure')\n",
    )]);
    let library = OptionalLibrary::default();
    let module = library
        .import(
            &host,
            "b8043_strict_library",
            "pip install b8043-strict-library",
        )
        .unwrap();
    let error = module.function(&host, "b8043_absent").unwrap_err();
    let LibraryError::Attribute {
        module: library_name,
        path,
        source,
    } = error
    else {
        panic!("expected an attribute failure, got {error:?}");
    };
    assert_eq!(library_name, "b8043_strict_library");
    assert_eq!(path, "b8043_absent");
    assert_eq!(source.type_name, "RuntimeError");
}

#[test]
fn function_refuses_an_attribute_that_is_not_callable() {
    let host = start_host(Vec::new());
    let library = OptionalLibrary::default();
    let os = library.import(&host, "os", "pip install os").unwrap();
    let error = os.function(&host, "path.sep").unwrap_err();
    assert!(matches!(
        error,
        LibraryError::NotCallable { module, path } if module == "os" && path == "path.sep"
    ));
}

#[tokio::test]
async fn call_blocking_runs_a_function_through_the_pool() {
    let (host, _folder) = host_with_files(&[(
        "b8043_blocking_library.py",
        "def total(values):\n    return sum(values)\n",
    )]);
    let library = OptionalLibrary::default();
    let module = library
        .import(
            &host,
            "b8043_blocking_library",
            "pip install b8043-blocking-library",
        )
        .unwrap();
    let total = module.function(&host, "total").unwrap();
    let value = module
        .call_blocking(
            &host,
            &tracker(),
            Duration::from_secs(5),
            BlockingCall {
                function: &total,
                args: vec![JsonValue::Array(vec![
                    JsonValue::from(1_i64),
                    JsonValue::from(2_i64),
                    JsonValue::from(3_i64),
                ])],
                kwargs: JsonObject::new(),
                new_temp_dir: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(value, JsonValue::from(6_i64));
}

#[tokio::test]
async fn call_blocking_passes_a_temporary_folder_that_lives_until_the_call_returns() {
    let (host, _folder) = host_with_files(&[(
        "b8043_temp_library.py",
        "import os\n\ndef inspect_dir(path):\n    return {'exists': os.path.isdir(path), 'path': path}\n",
    )]);
    let library = OptionalLibrary::default();
    let module = library
        .import(
            &host,
            "b8043_temp_library",
            "pip install b8043-temp-library",
        )
        .unwrap();
    let inspect = module.function(&host, "inspect_dir").unwrap();
    let value = module
        .call_blocking(
            &host,
            &tracker(),
            Duration::from_secs(5),
            BlockingCall {
                function: &inspect,
                args: Vec::new(),
                kwargs: JsonObject::new(),
                new_temp_dir: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(object_value(&value, "exists"), JsonValue::Bool(true));
    let JsonValue::String(path) = object_value(&value, "path") else {
        panic!("expected a path string, got {value:?}");
    };
    assert!(
        !Path::new(&path).exists(),
        "the temporary folder outlived the call: {path}"
    );
}

#[tokio::test]
async fn call_blocking_keeps_the_library_name_on_a_call_failure() {
    let (host, _folder) = host_with_files(&[(
        "b8043_failing_library.py",
        "def explode():\n    raise ValueError('boom')\n",
    )]);
    let library = OptionalLibrary::default();
    let module = library
        .import(
            &host,
            "b8043_failing_library",
            "pip install b8043-failing-library",
        )
        .unwrap();
    let explode = module.function(&host, "explode").unwrap();
    let error = module
        .call_blocking(
            &host,
            &tracker(),
            Duration::from_secs(5),
            BlockingCall {
                function: &explode,
                args: Vec::new(),
                kwargs: JsonObject::new(),
                new_temp_dir: false,
            },
        )
        .await
        .unwrap_err();
    let LibraryError::Call {
        module: library_name,
        source,
    } = error
    else {
        panic!("expected a call failure, got {error:?}");
    };
    assert_eq!(library_name, "b8043_failing_library");
    let CallError::Python(failure) = *source else {
        panic!("expected a Python error, got {source:?}");
    };
    assert_eq!(failure.type_name, "ValueError");
}

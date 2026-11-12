#![cfg(test)]

use std::error::Error as _;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _};
use pyo3::{Py, PyAny};

use super::{InstanceLoader, LoadError, LoadedObject};
use crate::blocking_work::WorkTracker;
use crate::host::{DEFAULT_SHUTDOWN_GRACE, HostConfig, PythonHost};
use crate::json_values::{JsonObject, JsonValue};
use crate::remote_module::{FetchError, ModuleFetcher, ModuleRequest};

struct NoFetch;

impl ModuleFetcher for NoFetch {
    fn fetch(&self, _request: &ModuleRequest) -> Result<(), FetchError> {
        Err(FetchError {
            message: "the loader tests download nothing".to_owned(),
        })
    }
}

fn host_and_loader(module_dirs: Vec<PathBuf>) -> (PythonHost, InstanceLoader) {
    let config = HostConfig {
        module_dirs,
        shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
    };
    let host = PythonHost::start(&config).unwrap();
    let loader = InstanceLoader::new(host.clone(), Arc::new(NoFetch));
    (host, loader)
}

fn loader_without_modules() -> InstanceLoader {
    host_and_loader(Vec::new()).1
}

fn object_text(host: &PythonHost, object: &Py<PyAny>) -> String {
    host.attach(|py| object.bind(py).extract::<String>().unwrap())
}

fn text_of(host: &PythonHost, instance: &LoadedObject) -> String {
    object_text(host, &instance.object)
}

#[tokio::test]
async fn loads_an_instance_from_a_file_next_to_the_config() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(
        folder.path().join("custom.py"),
        "instance = \"from-file\"\n",
    )
    .unwrap();
    let (host, loader) = host_and_loader(vec![folder.path().to_path_buf()]);
    let config = folder.path().join("config.yaml");
    let instance = loader.load("custom.instance", Some(&config)).await.unwrap();
    assert_eq!(text_of(&host, &instance), "from-file");
    assert_eq!(instance.value, "custom.instance");
}

#[tokio::test]
async fn loads_an_instance_from_sys_path_without_a_config_path() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(
        folder.path().join("b8042_path_module.py"),
        "instance = \"from-path\"\n",
    )
    .unwrap();
    let (host, loader) = host_and_loader(vec![folder.path().to_path_buf()]);
    let instance = loader
        .load("b8042_path_module.instance", None)
        .await
        .unwrap();
    assert_eq!(text_of(&host, &instance), "from-path");
}

#[tokio::test]
async fn a_file_next_to_the_config_wins_over_sys_path() {
    let config_folder = tempfile::tempdir().unwrap();
    let module_folder = tempfile::tempdir().unwrap();
    fs::write(
        config_folder.path().join("b8042_choice.py"),
        "instance = \"from-file\"\n",
    )
    .unwrap();
    fs::write(
        module_folder.path().join("b8042_choice.py"),
        "instance = \"from-path\"\n",
    )
    .unwrap();
    let (host, loader) = host_and_loader(vec![module_folder.path().to_path_buf()]);
    let config = config_folder.path().join("config.yaml");
    let instance = loader
        .load("b8042_choice.instance", Some(&config))
        .await
        .unwrap();
    assert_eq!(text_of(&host, &instance), "from-file");
}

#[tokio::test]
async fn a_module_file_runs_under_its_module_name_without_joining_sys_modules() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(folder.path().join("b8042_named.py"), "name = __name__\n").unwrap();
    let (host, loader) = host_and_loader(vec![]);
    let config = folder.path().join("config.yaml");
    let instance = loader
        .load("b8042_named.name", Some(&config))
        .await
        .unwrap();
    assert_eq!(text_of(&host, &instance), "b8042_named");
    let listed = host.attach(|py| {
        let modules = py.import("sys").unwrap().getattr("modules").unwrap();
        let table = modules.cast_into::<PyDict>().unwrap();
        table.contains("b8042_named").unwrap()
    });
    assert!(!listed, "the file module must not join sys.modules");
}

#[tokio::test]
async fn loads_a_module_file_below_a_subfolder_of_the_config_folder() {
    let folder = tempfile::tempdir().unwrap();
    fs::create_dir_all(folder.path().join("loggers")).unwrap();
    fs::write(
        folder.path().join("loggers/custom.py"),
        "instance = \"from-subfolder\"\n",
    )
    .unwrap();
    let (host, loader) = host_and_loader(vec![]);
    let config = folder.path().join("config.yaml");
    let instance = loader
        .load("loggers/custom.instance", Some(&config))
        .await
        .unwrap();
    assert_eq!(text_of(&host, &instance), "from-subfolder");
}

#[tokio::test]
async fn a_missing_module_is_reported_as_an_import_error() {
    let loader = loader_without_modules();
    let error = loader
        .load("b8042_no_such_module.instance", None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Could not import instance from b8042_no_such_module"
    );
    assert!(error.source().is_some());
    let LoadError::Import {
        module_name,
        instance_name,
        source,
    } = &error
    else {
        panic!("expected an import error, got {error:?}");
    };
    assert_eq!(module_name, "b8042_no_such_module");
    assert_eq!(instance_name, "instance");
    assert_eq!(source.type_name, "ModuleNotFoundError");
}

#[tokio::test]
async fn a_missing_module_without_an_instance_name_keeps_the_import_error() {
    let loader = loader_without_modules();
    let error = loader
        .load("b8042_no_such_module.", None)
        .await
        .unwrap_err();
    let LoadError::Python(failure) = &error else {
        panic!("expected the import error to pass through, got {error:?}");
    };
    assert_eq!(failure.type_name, "ModuleNotFoundError");
}

#[tokio::test]
async fn a_missing_attribute_is_a_passed_through_python_error() {
    let loader = loader_without_modules();
    let error = loader.load("json.b8042_missing", None).await.unwrap_err();
    let LoadError::Python(failure) = &error else {
        panic!("expected an attribute error, got {error:?}");
    };
    assert_eq!(failure.type_name, "AttributeError");
    assert!(
        failure.message.contains("b8042_missing"),
        "got: {}",
        failure.message
    );
}

#[tokio::test]
async fn a_value_without_a_module_part_imports_the_empty_module_name() {
    let loader = loader_without_modules();
    let error = loader.load("instance", None).await.unwrap_err();
    let LoadError::Python(failure) = &error else {
        panic!("expected the empty module name error, got {error:?}");
    };
    assert_eq!(failure.type_name, "ValueError");
    assert_eq!(failure.message, "Empty module name");
}

#[tokio::test]
async fn an_empty_module_part_is_refused_before_the_folder_is_read() {
    let folder = tempfile::tempdir().unwrap();
    let config_folder = folder.path().join("conf");
    fs::create_dir_all(&config_folder).unwrap();
    fs::write(
        config_folder.join("evil.py"),
        "raise RuntimeError('the guard must stop this file')\n",
    )
    .unwrap();
    let (_, loader) = host_and_loader(vec![]);
    let config = config_folder.join("config.yaml");
    let error = loader
        .load("../evil.instance", Some(&config))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Could not import instance from ../evil");
    assert!(matches!(error, LoadError::UnsafeModulePath { .. }));
}

#[tokio::test]
async fn an_absolute_module_part_is_refused_before_the_module_runs() {
    let folder = tempfile::tempdir().unwrap();
    let escape = folder.path().join("b8042_escape");
    fs::write(
        folder.path().join("b8042_escape.py"),
        "raise RuntimeError('the guard must stop this file')\n",
    )
    .unwrap();
    let (_, loader) = host_and_loader(vec![]);
    let value = format!("{}.instance", escape.display());
    let error = loader.load(&value, Some(folder.path())).await.unwrap_err();
    assert!(matches!(error, LoadError::UnsafeModulePath { .. }));
    assert!(
        !error.to_string().contains("RuntimeError"),
        "the module file ran: {error}"
    );
}

#[tokio::test]
async fn handle_returns_a_callable_for_a_loaded_function() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(
        folder.path().join("b8042_hook.py"),
        "def hook():\n    return 7\n",
    )
    .unwrap();
    let (host, loader) = host_and_loader(vec![]);
    let config = folder.path().join("config.yaml");
    let instance = loader.load("b8042_hook.hook", Some(&config)).await.unwrap();
    let handle = instance.handle(&host).unwrap();
    assert!(!handle.is_async);
    let tracker = WorkTracker::new(1, Duration::from_secs(10));
    let value = handle
        .call(
            &host,
            &tracker,
            Duration::from_secs(5),
            Vec::new(),
            JsonObject::new(),
        )
        .await
        .unwrap();
    assert_eq!(value, JsonValue::from(7_i64));
}

#[cfg(unix)]
#[tokio::test]
async fn a_module_file_path_that_is_not_utf8_is_reported() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let folder = tempfile::tempdir().unwrap();
    let strange = folder.path().join(OsStr::from_bytes(b"m\xff"));
    fs::create_dir_all(&strange).unwrap();
    fs::write(strange.join("custom.py"), "instance = 1\n").unwrap();
    let (_, loader) = host_and_loader(vec![]);
    let config = strange.join("config.yaml");
    let error = loader
        .load("custom.instance", Some(&config))
        .await
        .unwrap_err();
    let LoadError::PathNotUtf8 { path } = &error else {
        panic!("expected a non-UTF-8 path error, got {error:?}");
    };
    assert!(path.starts_with(&strange));
}

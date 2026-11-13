use std::path::{Path, PathBuf, is_separator};

use pyo3::exceptions::PyImportError;
use pyo3::types::PyAnyMethods as _;
use pyo3::{Bound, Py, PyAny, PyErr, Python};

use super::{LoadError, PythonFailure};
use crate::call_error::PythonError;

pub(crate) fn load_local(
    py: Python<'_>,
    module_name: &str,
    instance_name: &str,
    config_file_path: Option<&Path>,
) -> Result<Py<PyAny>, LoadError> {
    let file = local_module_file(config_file_path, module_name, instance_name)?;
    let module = import_module(py, module_name, file.as_deref())
        .map_err(|error| local_error(py, &error, module_name, instance_name))?;
    let instance = module
        .getattr(instance_name)
        .map_err(|error| local_error(py, &error, module_name, instance_name))?;
    Ok(instance.unbind())
}

fn local_module_file(
    config_file_path: Option<&Path>,
    module_name: &str,
    instance_name: &str,
) -> Result<Option<String>, LoadError> {
    let Some(config) = config_file_path else {
        return Ok(None);
    };
    if module_name.is_empty() {
        return Ok(None);
    }
    let segments = module_segments(module_name).ok_or_else(|| LoadError::UnsafeModulePath {
        module_name: module_name.to_owned(),
        instance_name: instance_name.to_owned(),
    })?;
    let mut path = config.parent().unwrap_or(config).to_path_buf();
    for segment in segments {
        path.push(segment);
    }
    let mut name = path.into_os_string();
    name.push(".py");
    let file = PathBuf::from(name);
    if !file.exists() {
        return Ok(None);
    }
    let Some(location) = file.to_str() else {
        return Err(LoadError::PathNotUtf8 { path: file });
    };
    Ok(Some(location.to_owned()))
}

fn module_segments(module_name: &str) -> Option<Vec<&str>> {
    module_name
        .split('.')
        .map(|segment| plain_segment(segment).then_some(segment))
        .collect()
}

fn plain_segment(segment: &str) -> bool {
    // a part that starts with a separator would make Path::push replace the config folder
    !segment.is_empty() && !segment.starts_with(is_separator)
}

fn import_module<'py>(
    py: Python<'py>,
    module_name: &str,
    file: Option<&str>,
) -> Result<Bound<'py, PyAny>, PyErr> {
    if let Some(location) = file {
        return load_module_file(py, module_name, location);
    }
    let importlib = py.import("importlib")?;
    importlib.call_method1("import_module", (module_name,))
}

fn load_module_file<'py>(
    py: Python<'py>,
    module_name: &str,
    location: &str,
) -> Result<Bound<'py, PyAny>, PyErr> {
    let spec = module_spec(py, module_name, location)?;
    if spec.is_none() {
        return Err(PyImportError::new_err(format!(
            "Could not find a module specification for {location}"
        )));
    }
    let util = py.import("importlib.util")?;
    let module = util.call_method1("module_from_spec", (&spec,))?;
    let loader = spec.getattr("loader")?;
    if loader.is_none() {
        return Err(PyImportError::new_err(format!(
            "Could not find a module loader for {location}"
        )));
    }
    loader.call_method1("exec_module", (&module,))?;
    Ok(module)
}

pub(crate) fn module_spec<'py>(
    py: Python<'py>,
    module_name: &str,
    location: &str,
) -> Result<Bound<'py, PyAny>, PyErr> {
    py.import("importlib.util")?
        .call_method1("spec_from_file_location", (module_name, location))
}

fn local_error(py: Python<'_>, error: &PyErr, module_name: &str, instance_name: &str) -> LoadError {
    if !module_name.is_empty()
        && !instance_name.is_empty()
        && error.is_instance_of::<PyImportError>(py)
    {
        return LoadError::Import {
            module_name: module_name.to_owned(),
            instance_name: instance_name.to_owned(),
            source: Box::new(PythonError::from_pyerr(py, error)),
        };
    }
    LoadError::Python(Box::new(PythonFailure::capture(py, error)))
}

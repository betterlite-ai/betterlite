#![expect(
    clippy::disallowed_types,
    reason = "Python hooks cross the interpreter as JSON trees, which is serde_json::Value here"
)]

use pyo3::exceptions::PyValueError;
use pyo3::types::{
    PyAnyMethods as _, PyBool, PyDict, PyDictMethods as _, PyFloat, PyInt, PyList,
    PyListMethods as _, PyString, PyTuple, PyTupleMethods as _, PyTypeMethods as _,
};
use pyo3::{Bound, IntoPyObjectExt as _, PyAny, PyErr, PyResult, Python};
use serde_json::Number;

pub type JsonValue = serde_json::Value;
pub type JsonObject = serde_json::Map<String, JsonValue>;

pub(crate) const MAX_DEPTH: usize = 64;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    #[error("a Python {type_name} value has no JSON form")]
    Unsupported { type_name: String },
    #[error("the value nests deeper than 64 levels")]
    TooDeep,
    #[error("a float that is NaN or infinite has no JSON form")]
    NonFinite,
}

impl From<ValueError> for PyErr {
    fn from(error: ValueError) -> Self {
        PyValueError::new_err(error.to_string())
    }
}

pub fn to_object<'py>(py: Python<'py>, value: &JsonValue) -> PyResult<Bound<'py, PyAny>> {
    match value {
        JsonValue::Null => Ok(py.None().into_bound(py)),
        JsonValue::Bool(flag) => flag.into_bound_py_any(py),
        JsonValue::Number(number) => number_to_object(py, number),
        JsonValue::String(text) => text.into_bound_py_any(py),
        JsonValue::Array(items) => array_to_object(py, items, 0),
        JsonValue::Object(entries) => map_to_object(py, entries, 0),
    }
}

pub fn from_object(object: &Bound<'_, PyAny>) -> Result<JsonValue, ValueError> {
    match container(object) {
        Some(Container::List(list)) => list_to_value(&list, 0),
        Some(Container::Tuple(tuple)) => tuple_to_value(&tuple, 0),
        Some(Container::Dict(dict)) => dict_to_value(&dict, 0),
        None => scalar_to_value(object),
    }
}

pub(crate) fn object_type_name(object: &Bound<'_, PyAny>) -> String {
    object
        .get_type()
        .name()
        .and_then(|name| name.extract::<String>())
        .unwrap_or_default()
}

fn nested_to_object<'py>(
    py: Python<'py>,
    value: &JsonValue,
    depth: usize,
) -> PyResult<Bound<'py, PyAny>> {
    if let JsonValue::Array(items) = value {
        return array_to_object(py, items, depth);
    }
    if let JsonValue::Object(entries) = value {
        return map_to_object(py, entries, depth);
    }
    to_object(py, value)
}

fn array_to_object<'py>(
    py: Python<'py>,
    items: &[JsonValue],
    depth: usize,
) -> PyResult<Bound<'py, PyAny>> {
    ensure_depth(depth).map_err(PyErr::from)?;
    let list = PyList::empty(py);
    for item in items {
        list.append(nested_to_object(py, item, depth.saturating_add(1))?)?;
    }
    Ok(list.into_any())
}

fn map_to_object<'py>(
    py: Python<'py>,
    entries: &JsonObject,
    depth: usize,
) -> PyResult<Bound<'py, PyAny>> {
    ensure_depth(depth).map_err(PyErr::from)?;
    let dict = PyDict::new(py);
    for (key, value) in entries {
        dict.set_item(key, nested_to_object(py, value, depth.saturating_add(1))?)?;
    }
    Ok(dict.into_any())
}

fn number_to_object<'py>(py: Python<'py>, number: &Number) -> PyResult<Bound<'py, PyAny>> {
    if let Some(value) = number.as_i64() {
        return value.into_bound_py_any(py);
    }
    if let Some(value) = number.as_u64() {
        return value.into_bound_py_any(py);
    }
    let value = number
        .as_f64()
        .ok_or(ValueError::NonFinite)
        .map_err(PyErr::from)?;
    value.into_bound_py_any(py)
}

fn list_to_value(object: &Bound<'_, PyList>, depth: usize) -> Result<JsonValue, ValueError> {
    ensure_depth(depth)?;
    let mut items = Vec::with_capacity(object.len());
    for item in object.iter() {
        items.push(read_child(&item, depth.saturating_add(1))?);
    }
    Ok(JsonValue::Array(items))
}

fn tuple_to_value(object: &Bound<'_, PyTuple>, depth: usize) -> Result<JsonValue, ValueError> {
    ensure_depth(depth)?;
    let mut items = Vec::with_capacity(object.len());
    for item in object.iter() {
        items.push(read_child(&item, depth.saturating_add(1))?);
    }
    Ok(JsonValue::Array(items))
}

fn dict_to_value(object: &Bound<'_, PyDict>, depth: usize) -> Result<JsonValue, ValueError> {
    ensure_depth(depth)?;
    let mut entries = JsonObject::new();
    for (key, value) in object.iter() {
        let text = read_key(&key)?;
        entries.insert(text, read_child(&value, depth.saturating_add(1))?);
    }
    Ok(JsonValue::Object(entries))
}

fn scalar_to_value(object: &Bound<'_, PyAny>) -> Result<JsonValue, ValueError> {
    if object.is_none() {
        return Ok(JsonValue::Null);
    }
    if object.is_instance_of::<PyBool>() {
        return read_bool(object);
    }
    if object.is_instance_of::<PyInt>() {
        return read_int(object);
    }
    if object.is_instance_of::<PyFloat>() {
        return read_float(object);
    }
    if object.is_instance_of::<PyString>() {
        return object
            .extract::<String>()
            .map(JsonValue::String)
            .map_err(|_error| unsupported(object));
    }
    Err(unsupported(object))
}

fn read_bool(object: &Bound<'_, PyAny>) -> Result<JsonValue, ValueError> {
    let flag = object
        .extract::<bool>()
        .map_err(|_error| unsupported(object))?;
    Ok(JsonValue::Bool(flag))
}

fn read_int(object: &Bound<'_, PyAny>) -> Result<JsonValue, ValueError> {
    if let Ok(value) = object.extract::<i64>() {
        return Ok(JsonValue::from(value));
    }
    if let Ok(value) = object.extract::<u64>() {
        return Ok(JsonValue::from(value));
    }
    Err(unsupported(object))
}

fn read_float(object: &Bound<'_, PyAny>) -> Result<JsonValue, ValueError> {
    let value = object
        .extract::<f64>()
        .map_err(|_error| unsupported(object))?;
    let number = Number::from_f64(value).ok_or(ValueError::NonFinite)?;
    Ok(JsonValue::Number(number))
}

fn read_key(key: &Bound<'_, PyAny>) -> Result<String, ValueError> {
    if !key.is_instance_of::<PyString>() {
        return Err(unsupported(key));
    }
    key.extract::<String>().map_err(|_error| unsupported(key))
}

fn read_child(object: &Bound<'_, PyAny>, depth: usize) -> Result<JsonValue, ValueError> {
    match container(object) {
        Some(Container::List(list)) => list_to_value(&list, depth),
        Some(Container::Tuple(tuple)) => tuple_to_value(&tuple, depth),
        Some(Container::Dict(dict)) => dict_to_value(&dict, depth),
        None => scalar_to_value(object),
    }
}

enum Container<'py> {
    List(Bound<'py, PyList>),
    Tuple(Bound<'py, PyTuple>),
    Dict(Bound<'py, PyDict>),
}

fn container<'py>(object: &Bound<'py, PyAny>) -> Option<Container<'py>> {
    if let Ok(list) = object.cast::<PyList>() {
        return Some(Container::List(list.clone()));
    }
    if let Ok(tuple) = object.cast::<PyTuple>() {
        return Some(Container::Tuple(tuple.clone()));
    }
    object
        .cast::<PyDict>()
        .ok()
        .map(|dict| Container::Dict(dict.clone()))
}

fn unsupported(object: &Bound<'_, PyAny>) -> ValueError {
    ValueError::Unsupported {
        type_name: object_type_name(object),
    }
}

const fn ensure_depth(depth: usize) -> Result<(), ValueError> {
    if depth >= MAX_DEPTH {
        return Err(ValueError::TooDeep);
    }
    Ok(())
}

mod tests;

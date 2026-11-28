use betterlite_openai::openai_base_config::Object;
use betterlite_pyhost::json_values::to_object;
use pyo3::exceptions::PyAttributeError;
use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _};
use pyo3::{Bound, Python};

use super::super::error::RivaCallError;
use super::{
    COPY_FROM, DEFAULT_ENCODING_ATTR, ENCODING_ATTR, ENCODING_FIELD, ENDPOINTING_ATTR,
    ENDPOINTING_FIELD, RECOGNITION, config_call, config_failure,
};
use crate::api::riva::records::recognition::RecognitionOptions;

pub(super) fn recognition_keywords<'py>(
    py: Python<'py>,
    module: &Bound<'py, PyAny>,
    options: &RecognitionOptions,
) -> Result<Bound<'py, PyDict>, RivaCallError> {
    let encoding = encoding_value(py, module, options)?;
    let keywords = PyDict::new(py);
    config_call(py, RECOGNITION, keywords.set_item(ENCODING_FIELD, encoding))?;
    for (name, value) in recognition_fields(py, options)? {
        config_call(py, RECOGNITION, keywords.set_item(name, value))?;
    }
    Ok(keywords)
}

// The constructor's keywords are the logged recognition keys, minus the enum and the endpointing message
fn recognition_fields<'py>(
    py: Python<'py>,
    options: &RecognitionOptions,
) -> Result<Vec<(String, Bound<'py, PyAny>)>, RivaCallError> {
    let encoded = serde_json::to_value(options).map_err(|source| RivaCallError::Json { source })?;
    let Some(fields) = encoded.as_object() else {
        return Err(RivaCallError::Malformed {
            message: RECOGNITION.to_owned(),
        });
    };
    let mut pairs = Vec::new();
    for (name, value) in fields {
        if name == ENCODING_FIELD || name == ENDPOINTING_FIELD {
            continue;
        }
        let field = config_call(py, RECOGNITION, to_object(py, value))?;
        pairs.push((name.clone(), field));
    }
    Ok(pairs)
}

fn encoding_value<'py>(
    py: Python<'py>,
    module: &Bound<'py, PyAny>,
    options: &RecognitionOptions,
) -> Result<Bound<'py, PyAny>, RivaCallError> {
    let encoding = config_call(py, RECOGNITION, module.getattr(ENCODING_ATTR))?;
    // the source reads the fallback eagerly, so a missing LINEAR_PCM fails even when the name exists
    let fallback = config_call(py, RECOGNITION, encoding.getattr(DEFAULT_ENCODING_ATTR))?;
    let name = options.encoding.to_uppercase();
    match encoding.getattr(name.as_str()) {
        Ok(value) => Ok(value),
        Err(error) if error.is_instance_of::<PyAttributeError>(py) => Ok(fallback),
        Err(error) => Err(config_failure(py, RECOGNITION, &error)),
    }
}

// Endpointing settings are best-effort: an older server rejects the fields, and the source ignores it
pub(super) fn apply_endpointing(
    py: Python<'_>,
    module: &Bound<'_, PyAny>,
    config: &Bound<'_, PyAny>,
    options: &RecognitionOptions,
) -> Result<(), RivaCallError> {
    let Some(endpointing) = endpointing_object(options)? else {
        return Ok(());
    };
    let Ok(factory) = module.getattr(ENDPOINTING_ATTR) else {
        return Ok(());
    };
    let keywords = PyDict::new(py);
    for (name, value) in &endpointing {
        let Ok(field) = to_object(py, value) else {
            return Ok(());
        };
        if keywords.set_item(name, field).is_err() {
            return Ok(());
        }
    }
    let Ok(message) = factory.call((), Some(&keywords)) else {
        return Ok(());
    };
    let Ok(target) = config.getattr(ENDPOINTING_FIELD) else {
        return Ok(());
    };
    drop(target.call_method1(COPY_FROM, (message,)));
    Ok(())
}

fn endpointing_object(options: &RecognitionOptions) -> Result<Option<Object>, RivaCallError> {
    let Some(config) = &options.endpointing_config else {
        return Ok(None);
    };
    let value = serde_json::to_value(config).map_err(|source| RivaCallError::Json { source })?;
    Ok(value
        .as_object()
        .cloned()
        .filter(|object| !object.is_empty()))
}

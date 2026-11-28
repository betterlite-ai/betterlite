use betterlite_models::decisions::Json;
use pyo3::types::{PyAnyMethods as _, PyStringMethods as _};
use pyo3::{Bound, PyAny, PyErr, Python};

use crate::api::riva::records::grpc_status::{
    CodeAttribute, DetailsAttribute, GrpcErrorObservation,
};

// What the SDK error exposed through code() and details(), read as the source's getattr does
pub(super) fn observe(py: Python<'_>, error: &PyErr) -> GrpcErrorObservation {
    let code = match call_attribute(py, error, "code") {
        AttributeCall::Missing => CodeAttribute::Missing,
        AttributeCall::NotCallable => CodeAttribute::NotCallable,
        AttributeCall::Raises => CodeAttribute::Raises,
        AttributeCall::Value(value) => CodeAttribute::Returns {
            name: name_json(&value),
        },
    };
    let details = match call_attribute(py, error, "details") {
        AttributeCall::Missing => DetailsAttribute::Missing,
        AttributeCall::NotCallable => DetailsAttribute::NotCallable,
        AttributeCall::Raises => DetailsAttribute::Raises,
        AttributeCall::Value(value) => DetailsAttribute::Returns {
            value: text_json(&value),
        },
    };
    let text = error.value(py).str().map_or_else(
        |_text| String::new(),
        |text| text.to_string_lossy().into_owned(),
    );
    GrpcErrorObservation {
        code,
        details,
        text,
    }
}

enum AttributeCall<'py> {
    Missing,
    NotCallable,
    Raises,
    Value(Bound<'py, PyAny>),
}

// error.value(py).<name>: a missing attribute, a non-callable one, a call that raises, or its value
fn call_attribute<'py>(py: Python<'py>, error: &PyErr, name: &str) -> AttributeCall<'py> {
    let Ok(attribute) = error.value(py).getattr(name) else {
        return AttributeCall::Missing;
    };
    if !attribute.is_callable() {
        return AttributeCall::NotCallable;
    }
    match attribute.call0() {
        Ok(value) => AttributeCall::Value(value),
        Err(_error) => AttributeCall::Raises,
    }
}

fn name_json(code: &Bound<'_, PyAny>) -> Json {
    match code
        .getattr("name")
        .and_then(|name| name.extract::<String>())
    {
        Ok(name) => Json::Text(name),
        Err(_error) => Json::Null,
    }
}

fn text_json(details: &Bound<'_, PyAny>) -> Json {
    match details.extract::<String>() {
        Ok(text) => Json::Text(text),
        Err(_error) => Json::Null,
    }
}

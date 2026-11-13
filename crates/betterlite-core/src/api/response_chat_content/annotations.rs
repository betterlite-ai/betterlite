use serde_json::Value;

use super::Projection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnnotationValue {
    Dict(Value),
    ModelDump(Value),
    LegacyDict(Value),
    Unsupported(String),
    Raises(String),
}

#[must_use]
pub fn annotation_fallback(annotations: Option<&[AnnotationValue]>) -> Projection {
    let mut results = Vec::new();
    let mut diagnostics = Vec::new();
    for annotation in annotations.unwrap_or_default() {
        match annotation {
            AnnotationValue::Dict(value)
            | AnnotationValue::ModelDump(value)
            | AnnotationValue::LegacyDict(value) => results.push(value.clone()),
            AnnotationValue::Unsupported(name) => diagnostics.push(format!(
                "Skipping unsupported annotation type: <class '{name}'>"
            )),
            AnnotationValue::Raises(error) => diagnostics.push(format!(
                "Skipping malformed annotation: annotation, error: {error}"
            )),
        }
    }
    Projection {
        value: if results.is_empty() {
            Value::Null
        } else {
            Value::Array(results)
        },
        diagnostics,
    }
}

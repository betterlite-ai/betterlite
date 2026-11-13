mod tests;

use serde_json::Value;
use thiserror::Error;

use crate::openai_base_config::Object;

const EXTRA_BODY: &str = "extra_body";
const INPUT_TYPE: &str = "input_type";
const TRUNCATE: &str = "truncate";

pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = ["encoding_format", "user", "dimensions"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NvidiaNimEmbeddingError {
    #[error("the extra_body parameter must be an object")]
    ExtraBodyNotObject,
}

#[must_use]
pub fn map_params(
    non_default_params: &Object,
    mut optional_params: Object,
    kwargs: Option<&Object>,
) -> Result<Object, NvidiaNimEmbeddingError> {
    optional_params
        .entry(EXTRA_BODY.to_owned())
        .or_insert_with(|| Value::Object(Object::new()));
    for (name, value) in non_default_params {
        if name == INPUT_TYPE || name == TRUNCATE {
            extra_body_object(&mut optional_params)?.insert(name.clone(), value.clone());
        } else {
            optional_params.insert(name.clone(), value.clone());
        }
    }
    if let Some(kwargs) = kwargs {
        let body = extra_body_object(&mut optional_params)?;
        for (name, value) in kwargs {
            body.insert(name.clone(), value.clone());
        }
    }
    Ok(optional_params)
}

fn extra_body_object(optional_params: &mut Object) -> Result<&mut Object, NvidiaNimEmbeddingError> {
    match optional_params.get_mut(EXTRA_BODY) {
        Some(Value::Object(fields)) => Ok(fields),
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_),
        )
        | None => Err(NvidiaNimEmbeddingError::ExtraBodyNotObject),
    }
}

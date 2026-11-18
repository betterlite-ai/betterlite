use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use thiserror::Error;

use crate::legacy_models::family::{ClassAttribute, ClassAttributes};

#[derive(Debug, Error)]
pub enum ConstructorError {
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
}

// The family views name their fields as the class dictionary keys, so one merge serves every constructor.
pub fn apply<T>(attributes: &mut ClassAttributes, values: &T) -> Result<(), ConstructorError>
where
    T: Serialize,
{
    let fields: IndexMap<String, Json> = serde_json::from_value(serde_json::to_value(values)?)?;
    for (name, value) in fields {
        if !matches!(value, Json::Null) {
            attributes.insert(name, ClassAttribute::Value(value));
        }
    }
    Ok(())
}

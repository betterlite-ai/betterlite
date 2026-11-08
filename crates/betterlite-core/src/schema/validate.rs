use super::bounds::{check_references, check_size};
use super::{SchemaError, SchemaJson, SchemaLimits};

// LiteLLM's validate_schema, bounded: it parses the response, compiles the schema offline and validates once
pub fn validate_schema(
    schema: &SchemaJson,
    response: &str,
    limits: &SchemaLimits,
) -> Result<(), SchemaError> {
    if response.len() > limits.max_response_bytes {
        return Err(SchemaError::ResponseTooLarge {
            size: response.len(),
            limit: limits.max_response_bytes,
        });
    }
    check_size(schema, limits)?;
    check_references(schema, limits)?;
    let instance: SchemaJson =
        serde_json::from_str(response).map_err(|source| SchemaError::InvalidJson { source })?;
    // Python's jsonschema only checks formats when a caller passes a format checker, and LiteLLM passes none
    let validator = jsonschema::options()
        .should_validate_formats(false)
        .offline()
        .build(schema)
        .map_err(|source| SchemaError::InvalidSchema { source })?;
    validator
        .validate(&instance)
        .map_err(|source| SchemaError::Mismatch {
            source: source.to_owned(),
        })?;
    Ok(())
}

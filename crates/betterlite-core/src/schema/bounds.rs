use super::{SchemaError, SchemaJson, SchemaLimits};

const SCALAR_BYTES: usize = 8;
const CONTAINER_BYTES: usize = 2;
const QUOTES: usize = 2;

// the schema is measured before anything reads it, so a huge document never reaches normalization or the validator
pub(super) fn check_size(schema: &SchemaJson, limits: &SchemaLimits) -> Result<(), SchemaError> {
    let mut pending = vec![schema];
    let mut nodes = 0_usize;
    let mut bytes = 0_usize;
    while let Some(value) = pending.pop() {
        nodes = nodes.saturating_add(1);
        if nodes > limits.max_nodes {
            return Err(SchemaError::TooManyNodes {
                limit: limits.max_nodes,
            });
        }
        add_bytes(&mut bytes, value_bytes(value), limits)?;
        match value {
            SchemaJson::Object(object) => {
                for (key, child) in object {
                    add_bytes(&mut bytes, key.len().saturating_add(QUOTES), limits)?;
                    pending.push(child);
                }
            }
            SchemaJson::Array(items) => pending.extend(items),
            SchemaJson::Null
            | SchemaJson::Bool(_)
            | SchemaJson::Number(_)
            | SchemaJson::String(_) => {}
        }
    }
    Ok(())
}

const fn add_bytes(
    bytes: &mut usize,
    amount: usize,
    limits: &SchemaLimits,
) -> Result<(), SchemaError> {
    *bytes = bytes.saturating_add(amount);
    if *bytes > limits.max_schema_bytes {
        return Err(SchemaError::SchemaTooLarge {
            limit: limits.max_schema_bytes,
        });
    }
    Ok(())
}

// every reference a validator may follow has to stay inside the document, so no fetch or file read can start
pub(super) fn check_references(
    schema: &SchemaJson,
    limits: &SchemaLimits,
) -> Result<(), SchemaError> {
    let mut pending = vec![(schema, 0_usize)];
    while let Some((value, depth)) = pending.pop() {
        match value {
            SchemaJson::Object(object) => {
                for (key, child) in object {
                    reject_external_reference(key, child)?;
                    push_child(&mut pending, child, depth, limits)?;
                }
            }
            SchemaJson::Array(items) => {
                for child in items {
                    push_child(&mut pending, child, depth, limits)?;
                }
            }
            SchemaJson::Null
            | SchemaJson::Bool(_)
            | SchemaJson::Number(_)
            | SchemaJson::String(_) => {}
        }
    }
    Ok(())
}

fn reject_external_reference(key: &str, value: &SchemaJson) -> Result<(), SchemaError> {
    if matches!(key, "$ref" | "$dynamicRef" | "$recursiveRef")
        && let SchemaJson::String(reference) = value
        && !reference.is_empty()
        && !reference.starts_with('#')
    {
        return Err(SchemaError::ExternalReference {
            reference: reference.clone(),
        });
    }
    Ok(())
}

fn push_child<'schema>(
    pending: &mut Vec<(&'schema SchemaJson, usize)>,
    value: &'schema SchemaJson,
    depth: usize,
    limits: &SchemaLimits,
) -> Result<(), SchemaError> {
    let next = depth.saturating_add(1);
    if next > limits.max_depth {
        return Err(SchemaError::SchemaTooDeep {
            limit: limits.max_depth,
        });
    }
    pending.push((value, next));
    Ok(())
}

const fn value_bytes(value: &SchemaJson) -> usize {
    match value {
        SchemaJson::String(text) => text.len().saturating_add(QUOTES),
        SchemaJson::Null | SchemaJson::Bool(_) | SchemaJson::Number(_) => SCALAR_BYTES,
        SchemaJson::Array(_) | SchemaJson::Object(_) => CONTAINER_BYTES,
    }
}

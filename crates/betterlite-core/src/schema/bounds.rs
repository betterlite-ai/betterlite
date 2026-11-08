use super::{SchemaError, SchemaJson, SchemaLimits};

const SCALAR_BYTES: usize = 8;
const CONTAINER_BYTES: usize = 2;
const QUOTES: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SchemaCheck {
    // normalization may truncate a deep schema and never follows a reference, so it only measures size
    Size,
    // validation also refuses a schema too deep to compile and any reference outside the document
    Validation,
}

struct Pending<'schema> {
    value: &'schema SchemaJson,
    key: Option<&'schema str>,
    depth: usize,
    // a value under const, enum, default or examples is an instance, never a schema
    data: bool,
}

// the schema is measured before anything reads it, so a huge document never reaches normalization or the validator
pub(super) fn check_schema(
    schema: &SchemaJson,
    limits: &SchemaLimits,
    check: SchemaCheck,
) -> Result<(), SchemaError> {
    let mut pending = vec![Pending {
        value: schema,
        key: None,
        depth: 0_usize,
        data: false,
    }];
    let mut nodes = 0_usize;
    let mut bytes = 0_usize;
    while let Some(entry) = pending.pop() {
        let Pending {
            value,
            key,
            depth,
            data,
        } = entry;
        nodes = nodes.saturating_add(1);
        if nodes > limits.max_nodes {
            return Err(SchemaError::TooManyNodes {
                limit: limits.max_nodes,
            });
        }
        add_bytes(&mut bytes, value_bytes(value), limits)?;
        if let Some(name) = key {
            add_bytes(&mut bytes, name.len().saturating_add(QUOTES), limits)?;
        }
        if check == SchemaCheck::Validation && !data {
            check_reference(value, key, depth, limits)?;
        }
        push_children(&mut pending, value, depth, data);
    }
    Ok(())
}

fn push_children<'schema>(
    pending: &mut Vec<Pending<'schema>>,
    value: &'schema SchemaJson,
    depth: usize,
    data: bool,
) {
    let next = depth.saturating_add(1);
    match value {
        SchemaJson::Object(object) => {
            for (child_key, child) in object {
                let child_data = data
                    || matches!(
                        child_key.as_str(),
                        "const" | "enum" | "default" | "examples"
                    );
                pending.push(Pending {
                    value: child,
                    key: Some(child_key.as_str()),
                    depth: next,
                    data: child_data,
                });
            }
        }
        SchemaJson::Array(items) => {
            for child in items {
                pending.push(Pending {
                    value: child,
                    key: None,
                    depth: next,
                    data,
                });
            }
        }
        SchemaJson::Null | SchemaJson::Bool(_) | SchemaJson::Number(_) | SchemaJson::String(_) => {}
    }
}

fn check_reference(
    value: &SchemaJson,
    key: Option<&str>,
    depth: usize,
    limits: &SchemaLimits,
) -> Result<(), SchemaError> {
    if depth > limits.max_depth {
        return Err(SchemaError::SchemaTooDeep {
            limit: limits.max_depth,
        });
    }
    key.map_or_else(|| Ok(()), |name| reject_external_reference(name, value))
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

const fn value_bytes(value: &SchemaJson) -> usize {
    match value {
        SchemaJson::String(text) => text.len().saturating_add(QUOTES),
        SchemaJson::Null | SchemaJson::Bool(_) | SchemaJson::Number(_) => SCALAR_BYTES,
        SchemaJson::Array(_) | SchemaJson::Object(_) => CONTAINER_BYTES,
    }
}

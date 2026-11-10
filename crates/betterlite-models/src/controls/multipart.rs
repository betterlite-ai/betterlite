use indexmap::IndexMap;

use crate::controls::json_value::JsonValue;

pub const DEFAULT_MAX_RECURSE_DEPTH: usize = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormValue {
    Single(String),
    Repeated(Vec<String>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormField {
    pub name: String,
    pub value: FormValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultipartField {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum FormFieldError {
    #[error("form field nesting exceeds {limit}")]
    TooDeep { limit: usize },
    #[error("a form source must be a JSON object")]
    NotAnObject,
}

// A list whose entries are all scalars stays one field carrying every element, so a repeated part is not lost under dict.update.
pub fn flatten_form_field_values(
    sources: &[Option<&JsonValue>],
    max_depth: usize,
) -> Result<Vec<FormField>, FormFieldError> {
    let mut fields = Vec::new();
    for source in sources.iter().flatten() {
        let JsonValue::Object(entries) = source else {
            return Err(FormFieldError::NotAnObject);
        };
        for (name, value) in entries {
            flatten_form_data_field(name, value, max_depth, &mut fields)?;
        }
    }
    Ok(fields)
}

// Every list element becomes its own name[] part, the way the OpenAI SDK serializes a multipart body.
pub fn serialize_multipart_fields(
    data: &JsonValue,
    max_depth: usize,
) -> Result<Vec<MultipartField>, FormFieldError> {
    let JsonValue::Object(entries) = data else {
        return Err(FormFieldError::NotAnObject);
    };
    let mut fields = Vec::new();
    for (name, value) in entries {
        flatten_form_field(name, value, max_depth, &mut fields)?;
    }
    Ok(fields)
}

fn flatten_form_data_field(
    name: &str,
    value: &JsonValue,
    max_depth: usize,
    fields: &mut Vec<FormField>,
) -> Result<(), FormFieldError> {
    let mut pending: Vec<(String, &JsonValue, usize)> = vec![(name.to_owned(), value, 0)];
    while let Some((current_name, current_value, depth)) = pending.pop() {
        if depth > max_depth {
            return Err(FormFieldError::TooDeep { limit: max_depth });
        }
        match current_value {
            JsonValue::Object(entries) => {
                pending.extend(object_children(&current_name, entries, depth));
            }
            JsonValue::Array(items) => {
                let all_scalars = items.iter().all(|item| {
                    !matches!(
                        item,
                        JsonValue::Null | JsonValue::Object(_) | JsonValue::Array(_)
                    )
                });
                if all_scalars {
                    push_repeated_field(current_name, items, fields);
                } else {
                    pending.extend(list_children(&current_name, items, depth));
                }
            }
            JsonValue::Null => {}
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) => {
                push_single_field(&current_name, current_value, fields);
            }
        }
    }
    Ok(())
}

fn flatten_form_field(
    name: &str,
    value: &JsonValue,
    max_depth: usize,
    fields: &mut Vec<MultipartField>,
) -> Result<(), FormFieldError> {
    let mut pending: Vec<(String, &JsonValue, usize)> = vec![(name.to_owned(), value, 0)];
    while let Some((current_name, current_value, depth)) = pending.pop() {
        if depth > max_depth {
            return Err(FormFieldError::TooDeep { limit: max_depth });
        }
        match current_value {
            JsonValue::Object(entries) => {
                pending.extend(object_children(&current_name, entries, depth));
            }
            JsonValue::Array(items) => {
                pending.extend(list_children(&current_name, items, depth));
            }
            JsonValue::Null => {}
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) => {
                if let Some(serialized) = form_scalar_text(current_value)
                    && !serialized.is_empty()
                {
                    fields.push(MultipartField {
                        name: current_name,
                        value: serialized,
                    });
                }
            }
        }
    }
    Ok(())
}

fn object_children<'values>(
    name: &str,
    entries: &'values IndexMap<String, JsonValue>,
    depth: usize,
) -> Vec<(String, &'values JsonValue, usize)> {
    let mut children = Vec::new();
    for (key, child) in entries.iter().rev() {
        children.push((format!("{name}[{key}]"), child, depth.saturating_add(1)));
    }
    children
}

fn list_children<'values>(
    name: &str,
    items: &'values [JsonValue],
    depth: usize,
) -> Vec<(String, &'values JsonValue, usize)> {
    let mut children = Vec::new();
    for item in items.iter().rev() {
        children.push((format!("{name}[]"), item, depth.saturating_add(1)));
    }
    children
}

fn push_repeated_field(name: String, items: &[JsonValue], fields: &mut Vec<FormField>) {
    let serialized: Vec<String> = items
        .iter()
        .filter_map(form_scalar_text)
        .filter(|text| !text.is_empty())
        .collect();
    if !serialized.is_empty() {
        fields.push(FormField {
            name,
            value: FormValue::Repeated(serialized),
        });
    }
}

fn push_single_field(name: &str, value: &JsonValue, fields: &mut Vec<FormField>) {
    if let Some(serialized) = form_scalar_text(value)
        && !serialized.is_empty()
    {
        fields.push(FormField {
            name: name.to_owned(),
            value: FormValue::Single(serialized),
        });
    }
}

fn form_scalar_text(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::Bool(true) => Some("true".to_owned()),
        JsonValue::Bool(false) => Some("false".to_owned()),
        JsonValue::Number(number) => Some(number.to_string()),
        JsonValue::Text(text) => Some(text.clone()),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

mod tests;

use crate::controls::json_value::JsonValue;

pub const DEFAULT_MAX_RECURSE_DEPTH: usize = 100;
pub const MAX_FORM_FIELD_BYTES: usize = 10_000_000;

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
    #[error("form fields exceed the {limit} byte limit")]
    TooLarge { limit: usize },
    #[error("a form source must be a JSON object")]
    NotAnObject,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ListStyle {
    Repeated,
    Expanded,
}

// A list whose entries are all scalars stays one field carrying every element, so a repeated part is not lost under dict.update.
pub fn flatten_form_field_values(
    sources: &[Option<&JsonValue>],
    max_depth: usize,
) -> Result<Vec<FormField>, FormFieldError> {
    let mut flattener = Flattener {
        max_depth,
        lists: ListStyle::Repeated,
        fields: Vec::new(),
        pending: Vec::new(),
        used: 0,
    };
    for source in sources.iter().flatten() {
        let JsonValue::Object(entries) = source else {
            return Err(FormFieldError::NotAnObject);
        };
        for (name, value) in entries {
            flattener.flatten_root(name, value)?;
        }
    }
    Ok(flattener.fields)
}

// Every list element becomes its own name[] part, the way the OpenAI SDK serializes a multipart body.
pub fn serialize_multipart_fields(
    data: &JsonValue,
    max_depth: usize,
) -> Result<Vec<MultipartField>, FormFieldError> {
    let JsonValue::Object(entries) = data else {
        return Err(FormFieldError::NotAnObject);
    };
    let mut flattener = Flattener {
        max_depth,
        lists: ListStyle::Expanded,
        fields: Vec::new(),
        pending: Vec::new(),
        used: 0,
    };
    for (name, value) in entries {
        flattener.flatten_root(name, value)?;
    }
    let mut serialized = Vec::new();
    for field in flattener.fields {
        match field.value {
            FormValue::Single(value) => serialized.push(MultipartField {
                name: field.name,
                value,
            }),
            FormValue::Repeated(values) => {
                let name = format!("{}[]", field.name);
                for value in values {
                    serialized.push(MultipartField {
                        name: name.clone(),
                        value,
                    });
                }
            }
        }
    }
    Ok(serialized)
}

struct Flattener<'value> {
    max_depth: usize,
    lists: ListStyle,
    fields: Vec<FormField>,
    pending: Vec<(String, &'value JsonValue, usize)>,
    used: usize,
}

impl<'value> Flattener<'value> {
    fn flatten_root(&mut self, name: &str, value: &'value JsonValue) -> Result<(), FormFieldError> {
        self.charge(name.len())?;
        self.pending.push((name.to_owned(), value, 0));
        while let Some((field, current, depth)) = self.pending.pop() {
            if depth > self.max_depth {
                return Err(FormFieldError::TooDeep {
                    limit: self.max_depth,
                });
            }
            match current {
                JsonValue::Object(entries) => self.push_children(
                    &field,
                    depth,
                    entries
                        .iter()
                        .rev()
                        .map(|(key, child)| (key.as_str(), child)),
                )?,
                JsonValue::Array(items) => self.push_array(field, items, depth)?,
                JsonValue::Null => {}
                JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) => {
                    self.push_scalar(field, current)?;
                }
            }
        }
        Ok(())
    }

    fn push_scalar(&mut self, name: String, value: &JsonValue) -> Result<(), FormFieldError> {
        if let Some(text) = form_scalar_text(value)
            && !text.is_empty()
        {
            self.charge(text.len())?;
            self.fields.push(FormField {
                name,
                value: FormValue::Single(text),
            });
        }
        Ok(())
    }

    fn push_array(
        &mut self,
        name: String,
        items: &'value [JsonValue],
        depth: usize,
    ) -> Result<(), FormFieldError> {
        let all_scalars = items.iter().all(|item| {
            !matches!(
                item,
                JsonValue::Null | JsonValue::Object(_) | JsonValue::Array(_)
            )
        });
        if all_scalars && self.lists == ListStyle::Repeated {
            let values: Vec<String> = items
                .iter()
                .filter_map(form_scalar_text)
                .filter(|text| !text.is_empty())
                .collect();
            if !values.is_empty() {
                let value_bytes = values
                    .iter()
                    .fold(0_usize, |total, value| total.saturating_add(value.len()));
                self.charge(value_bytes)?;
                self.fields.push(FormField {
                    name,
                    value: FormValue::Repeated(values),
                });
            }
            return Ok(());
        }
        self.push_children(&name, depth, items.iter().rev().map(|item| ("", item)))
    }

    fn push_children<'child, I>(
        &mut self,
        name: &str,
        depth: usize,
        children: I,
    ) -> Result<(), FormFieldError>
    where
        I: DoubleEndedIterator<Item = (&'child str, &'value JsonValue)>,
    {
        for (suffix, child) in children {
            let name_bytes = name.len().saturating_add(suffix.len()).saturating_add(2);
            self.charge(name_bytes)?;
            self.pending
                .push((format!("{name}[{suffix}]"), child, depth.saturating_add(1)));
        }
        Ok(())
    }

    const fn charge(&mut self, bytes: usize) -> Result<(), FormFieldError> {
        let total = self.used.saturating_add(bytes);
        if total > MAX_FORM_FIELD_BYTES {
            return Err(FormFieldError::TooLarge {
                limit: MAX_FORM_FIELD_BYTES,
            });
        }
        self.used = total;
        Ok(())
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

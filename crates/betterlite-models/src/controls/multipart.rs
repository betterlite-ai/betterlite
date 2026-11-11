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
    };
    for source in sources.iter().flatten() {
        let JsonValue::Object(entries) = source else {
            return Err(FormFieldError::NotAnObject);
        };
        flattener.seed(entries);
    }
    flattener.run()?;
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
    };
    flattener.seed(entries);
    flattener.run()?;
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
}

impl<'value> Flattener<'value> {
    fn seed(&mut self, entries: &'value IndexMap<String, JsonValue>) {
        for (name, value) in entries {
            self.pending.push((name.clone(), value, 0));
        }
    }

    fn run(&mut self) -> Result<(), FormFieldError> {
        while let Some((name, value, depth)) = self.pending.pop() {
            if depth > self.max_depth {
                return Err(FormFieldError::TooDeep {
                    limit: self.max_depth,
                });
            }
            match value {
                JsonValue::Object(entries) => self.push_children(
                    &name,
                    depth,
                    entries
                        .iter()
                        .rev()
                        .map(|(key, child)| (key.as_str(), child)),
                ),
                JsonValue::Array(items) => self.push_array(name, items, depth),
                JsonValue::Null => {}
                JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) => {
                    self.push_scalar(name, value);
                }
            }
        }
        Ok(())
    }

    fn push_scalar(&mut self, name: String, value: &JsonValue) {
        if let Some(text) = form_scalar_text(value)
            && !text.is_empty()
        {
            self.fields.push(FormField {
                name,
                value: FormValue::Single(text),
            });
        }
    }

    fn push_array(&mut self, name: String, items: &'value [JsonValue], depth: usize) {
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
                self.fields.push(FormField {
                    name,
                    value: FormValue::Repeated(values),
                });
            }
            return;
        }
        self.push_children(&name, depth, items.iter().rev().map(|item| ("", item)));
    }

    fn push_children<'child, I>(&mut self, name: &str, depth: usize, children: I)
    where
        I: DoubleEndedIterator<Item = (&'child str, &'value JsonValue)>,
    {
        for (suffix, child) in children {
            self.pending
                .push((format!("{name}[{suffix}]"), child, depth.saturating_add(1)));
        }
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

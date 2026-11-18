use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use thiserror::Error;

// The upload a multipart part carried, read once at decode time; Python's UploadFile exposes the same filename, content_type, size and read() content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadFile {
    pub filename: String,
    pub content_type: Option<String>,
    pub content: Vec<u8>,
    pub size: Option<u64>,
}

// Python's float() values that JSON cannot carry; the wire projection names them rather than emitting an invalid number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NonFinite {
    Nan,
    PositiveInfinity,
    NegativeInfinity,
}

impl NonFinite {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Nan => "nan",
            Self::PositiveInfinity => "positive_infinity",
            Self::NegativeInfinity => "negative_infinity",
        }
    }
}

// One decoded field value: a form yields text, uploads and the key[] lists, and coercion turns an eligible text into a number.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormValue {
    Text(String),
    Upload(UploadFile),
    List(Vec<FormValue>),
    Number(Number),
    Other(Json),
    NonFinite(NonFinite),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PartValue {
    Text(String),
    Upload(UploadFile),
}

// One entry of FormData.multi_items(): repeats stay separate until get_form_data picks the last plain value or a key[] list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormPart {
    pub name: String,
    pub value: PartValue,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FormParts {
    pub items: Vec<FormPart>,
}

pub type FormFields = IndexMap<String, FormValue>;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum FormError {
    #[error(
        "the form field {name} already holds a scalar, so a {name}[] entry cannot append to it"
    )]
    ScalarAppend { name: String },
}

// get_form_data: a plain key overwrites whatever came before, a key[] entry appends to its list, and a list key that already holds a scalar fails the way list.append on a str does.
pub fn get_form_data(parts: &FormParts) -> Result<FormFields, FormError> {
    let mut fields = FormFields::new();
    for part in &parts.items {
        let value = part_value(&part.value);
        match part.name.strip_suffix("[]") {
            Some(base) => append_to_list(&mut fields, base, value)?,
            None => {
                fields.insert(part.name.clone(), value);
            }
        }
    }
    Ok(fields)
}

fn part_value(value: &PartValue) -> FormValue {
    match value {
        PartValue::Text(text) => FormValue::Text(text.clone()),
        PartValue::Upload(upload) => FormValue::Upload(upload.clone()),
    }
}

fn append_to_list(fields: &mut FormFields, base: &str, value: FormValue) -> Result<(), FormError> {
    match fields.get_mut(base) {
        Some(FormValue::List(items)) => items.push(value),
        Some(_scalar) => {
            return Err(FormError::ScalarAppend {
                name: base.to_owned(),
            });
        }
        None => {
            fields.insert(base.to_owned(), FormValue::List(vec![value]));
        }
    }
    Ok(())
}

use serde_json::Value;

#[derive(Debug)]
pub struct FormModel {
    pub ui_friendly_name: Option<&'static str>,
    pub fields: Vec<FormField>,
}

#[derive(Debug)]
pub struct FormField {
    pub name: &'static str,
    pub description: Option<&'static str>,
    pub required: bool,
    pub shape: FormShape,
    pub default: Option<Value>,
    pub extra: FormExtra,
}

#[derive(Debug)]
pub enum FormShape {
    Text,
    Number,
    Boolean,
    Mapping {
        keys: Vec<Value>,
        value: Box<FormShape>,
    },
    List {
        options: Vec<Value>,
    },
    Choice(Vec<Value>),
    Nested(Box<FormModel>),
    RawObject,
    RawList,
    Other,
}

#[derive(Debug)]
pub struct FormExtra {
    pub ui_type: Option<String>,
    pub type_override: Option<String>,
    pub options: Option<Vec<Value>>,
    pub min: Option<Value>,
    pub max: Option<Value>,
    pub step: Option<Value>,
    pub default_value: Option<Value>,
}

impl FormExtra {
    pub const NONE: Self = Self {
        ui_type: None,
        type_override: None,
        options: None,
        min: None,
        max: None,
        step: None,
        default_value: None,
    };
}

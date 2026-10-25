use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::lax_bool::ScimBool;
use super::raw_object::RawObject;

// entitlements and roles are directory attributes kept in metadata; they grant no proxy role
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct DirectoryAttribute {
    #[serde(flatten)]
    pub fields: AttributeFields,
    #[serde(flatten)]
    pub extra: RawObject,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributeFields {
    pub value: Option<String>,
    pub display: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub primary: Option<ScimBool>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum AttributeShape {
    Bare(String),
    Object(AttributeFields),
}
impl TryFrom<Box<RawValue>> for DirectoryAttribute {
    type Error = serde_json::Error;
    fn try_from(raw: Box<RawValue>) -> Result<Self, serde_json::Error> {
        match serde_json::from_str(raw.get())? {
            AttributeShape::Bare(value) => Ok(Self {
                fields: AttributeFields {
                    value: Some(value),
                    display: None,
                    kind: None,
                    primary: None,
                },
                extra: RawObject::default(),
            }),
            AttributeShape::Object(fields) => {
                let mut extra: RawObject = serde_json::from_str(raw.get())?;
                extra.0.retain(|name, _| {
                    !matches!(name.as_str(), "value" | "display" | "type" | "primary")
                });
                Ok(Self { fields, extra })
            }
        }
    }
}

use serde::Deserialize;
use serde_json::value::RawValue;

use super::RawObject;
use super::user::ScimBool;

// entitlements and roles are directory attributes kept in metadata; they grant no proxy role
record! { #[serde(try_from = "Box<RawValue>")] DirectoryAttribute {
    #[serde(flatten)] pub fields: AttributeFields,
    #[serde(flatten)] pub extra: RawObject,
}}
record! { AttributeFields {
    pub value: Option<String>,
    pub display: Option<String>,
    #[serde(rename = "type")] pub kind: Option<String>,
    pub primary: Option<ScimBool>,
}}
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
                extra: RawObject::new(),
            }),
            AttributeShape::Object(fields) => {
                let mut extra: RawObject = serde_json::from_str(raw.get())?;
                extra.retain(|name, _| {
                    !matches!(name.as_str(), "value" | "display" | "type" | "primary")
                });
                Ok(Self { fields, extra })
            }
        }
    }
}

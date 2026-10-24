use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum RawMetadataField {
    Text(String),
    Integer(i64),
    Boolean(bool),
    Null,
}

// LiteLLM runs str() on every member, so YAML scalars keep Python's spelling
pub(crate) fn metadata_fields(raw: Option<Vec<RawMetadataField>>) -> Vec<String> {
    let mut fields = Vec::new();
    for field in raw.unwrap_or_default() {
        fields.push(match field {
            RawMetadataField::Text(text) => text,
            RawMetadataField::Integer(number) => number.to_string(),
            RawMetadataField::Boolean(true) => "True".to_owned(),
            RawMetadataField::Boolean(false) => "False".to_owned(),
            RawMetadataField::Null => "None".to_owned(),
        });
    }
    fields
}

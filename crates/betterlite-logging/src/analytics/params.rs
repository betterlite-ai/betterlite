use super::python::{FormatError, repr};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Primitive {
    Text(String),
    Boolean(bool),
    Number(serde_json::Number),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Parameter<Structured> {
    Primitive(Primitive),
    Structured(Structured),
    Null,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Params<Structured, Tools> {
    pub tools: Option<Tools>,
    #[serde(flatten)]
    pub values: BTreeMap<String, Parameter<Structured>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Cleaned<Tools> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Tools>,
    #[serde(flatten)]
    pub values: BTreeMap<String, Primitive>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Headers {
    #[serde(rename = "Template-Id")]
    pub template_id: Option<String>,
    #[serde(flatten)]
    pub other: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Extra<Structured, Tools, Body> {
    pub extra_body: Option<Body>,
    pub user: Option<String>,
    pub extra_headers: Option<Headers>,
    #[serde(flatten)]
    pub params: Params<Structured, Tools>,
}

pub fn clean_extra_params<Structured, Tools, Body>(
    extra: &Extra<Structured, Tools, Body>,
    optional: &Extra<Structured, Tools, Body>,
) -> Result<(Cleaned<Tools>, Option<String>), FormatError>
where
    Structured: Serialize + Clone,
    Tools: Clone,
{
    let mut merged = extra.params.values.clone();
    merged.extend(optional.params.values.clone());
    let values = merged
        .iter()
        .map(|(key, value)| {
            let cleaned = match value {
                Parameter::Primitive(primitive) => primitive.clone(),
                Parameter::Structured(structured) => Primitive::Text(repr(structured)?),
                Parameter::Null => Primitive::Text("None".to_owned()),
            };
            Ok((key.clone(), cleaned))
        })
        .collect::<Result<BTreeMap<_, _>, FormatError>>()?;
    let headers = optional
        .extra_headers
        .as_ref()
        .or(extra.extra_headers.as_ref());
    let template = headers.and_then(|value| value.template_id.clone());
    Ok((
        Cleaned {
            tools: optional
                .params
                .tools
                .clone()
                .or_else(|| extra.params.tools.clone()),
            values,
        },
        template,
    ))
}

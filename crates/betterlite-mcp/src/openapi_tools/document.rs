use std::fmt::{self, Formatter};

use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};

use crate::openapi_tools::error::OpenApiError;

// The subset of an OpenAPI or Swagger document betterlite reads, with paths and methods kept in file order
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OpenApiDocument {
    #[serde(default, deserialize_with = "absent_or_null")]
    pub servers: Vec<Server>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default, deserialize_with = "absent_or_null")]
    pub schemes: Vec<String>,
    #[serde(rename = "basePath", default)]
    pub base_path: Option<String>,
    #[serde(default, deserialize_with = "absent_or_null")]
    pub paths: IndexMap<String, PathItem>,
    #[serde(default, deserialize_with = "absent_or_null")]
    pub components: Components,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Server {
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Components {
    #[serde(default, deserialize_with = "absent_or_null")]
    pub parameters: IndexMap<String, Parameter>,
}

#[derive(Debug, Clone, Default)]
pub struct PathItem {
    pub parameters: Vec<Parameter>,
    pub operations: IndexMap<String, Operation>,
}

impl<'de> Deserialize<'de> for PathItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(PathItemVisitor)
    }
}

struct PathItemVisitor;

impl<'de> Visitor<'de> for PathItemVisitor {
    type Value = PathItem;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("an OpenAPI path item object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut item = PathItem::default();
        while let Some(key) = map.next_key::<String>()? {
            if key == "parameters" {
                let parameters = map.next_value::<Option<Vec<Parameter>>>()?;
                item.parameters = parameters.unwrap_or_default();
            } else if matches!(
                key.as_str(),
                "get" | "put" | "post" | "delete" | "options" | "head" | "patch" | "trace"
            ) {
                let operation = map.next_value::<Operation>()?;
                item.operations.insert(key, operation);
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(item)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Operation {
    #[serde(rename = "operationId", default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "absent_or_null")]
    pub parameters: Vec<Parameter>,
    #[serde(rename = "requestBody", default)]
    pub request_body: Option<RequestBody>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Parameter {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "in", default)]
    pub location: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: Option<bool>,
    #[serde(default)]
    pub schema: Option<Schema>,
    #[serde(rename = "$ref", default)]
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RequestBody {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: Option<bool>,
    #[serde(default, deserialize_with = "absent_or_null")]
    pub content: IndexMap<String, MediaType>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MediaType {
    #[serde(default)]
    pub schema: Option<Schema>,
}

// Only the fields a tool schema copies are modelled; $ref, allOf, items and required stay out as in LiteLLM
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Schema {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "absent_or_null")]
    pub properties: Object,
}

pub fn parse_openapi_document(bytes: &[u8]) -> Result<OpenApiDocument, OpenApiError> {
    let document = serde_json::from_slice(bytes).map_err(|source| OpenApiError::Json { source })?;
    Ok(document)
}

fn absent_or_null<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let Some(value) = Option::<T>::deserialize(deserializer)? else {
        return Ok(T::default());
    };
    Ok(value)
}

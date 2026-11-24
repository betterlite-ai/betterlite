use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_with::{As, DefaultOnNull};
use thiserror::Error;

use crate::openapi_tools::error::OpenApiError;

// The subset of an OpenAPI or Swagger document betterlite reads, with paths and methods kept in file order
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OpenApiDocument {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub servers: Vec<Server>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub schemes: Vec<String>,
    #[serde(rename = "basePath", default)]
    pub base_path: Option<String>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub paths: IndexMap<String, PathItem>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub components: Components,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Server {
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Components {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub parameters: IndexMap<String, Parameter>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(try_from = "RawPathItem")]
pub struct PathItem {
    pub parameters: Vec<Parameter>,
    pub operations: IndexMap<String, Operation>,
}

#[derive(Debug, Error)]
pub enum PathItemError {
    #[error("the OpenAPI path item cannot read its {method:?} operation as an object")]
    Operation { method: String },
}

// IndexMap keeps the order the document lists the methods in, so tools come out in file order
#[derive(Debug, Deserialize)]
struct RawPathItem {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    parameters: Vec<Parameter>,
    #[serde(flatten)]
    entries: IndexMap<String, PathEntry>,
}

// a path item also carries $ref, summary and x- extensions; the ones that are not operations are dropped
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PathEntry {
    Operation(Operation),
    Other(IgnoredAny),
}

impl TryFrom<RawPathItem> for PathItem {
    type Error = PathItemError;

    fn try_from(raw: RawPathItem) -> Result<Self, Self::Error> {
        let mut operations = IndexMap::new();
        for (method, entry) in raw.entries {
            if !matches!(
                method.as_str(),
                "get" | "put" | "post" | "delete" | "options" | "head" | "patch" | "trace"
            ) {
                continue;
            }
            match entry {
                PathEntry::Operation(operation) => {
                    operations.insert(method, operation);
                }
                PathEntry::Other(_ignored) => {
                    return Err(PathItemError::Operation { method });
                }
            }
        }
        Ok(Self {
            parameters: raw.parameters,
            operations,
        })
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
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
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
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
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
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub properties: Object,
}

pub fn parse_openapi_document(bytes: &[u8]) -> Result<OpenApiDocument, OpenApiError> {
    let document: OpenApiDocument =
        serde_json::from_slice(bytes).map_err(|source| OpenApiError::Json { source })?;
    Ok(document)
}

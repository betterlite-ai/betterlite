use serde::Serialize;

use super::{EF_SEARCH, TEXT_TYPE, VECTOR_DIMENSION, VECTOR_TYPE};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexBody {
    pub settings: IndexSettings,
    pub mappings: IndexMappings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexSettings {
    pub index: IndexKnobs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexKnobs {
    pub knn: bool,
    #[serde(rename = "knn.algo_param.ef_search")]
    pub ef_search: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexMappings {
    pub properties: IndexProperties,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexProperties {
    #[serde(rename = "bedrock-knowledge-base-default-vector")]
    pub vector: VectorField,
    #[serde(rename = "AMAZON_BEDROCK_METADATA")]
    pub metadata: MetadataField,
    #[serde(rename = "AMAZON_BEDROCK_TEXT_CHUNK")]
    pub text_chunk: TextField,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VectorField {
    #[serde(rename = "type")]
    pub field_type: String,
    pub dimension: u32,
    pub method: VectorMethod,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VectorMethod {
    pub engine: String,
    pub name: String,
    pub space_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MetadataField {
    #[serde(rename = "type")]
    pub field_type: String,
    pub index: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextField {
    #[serde(rename = "type")]
    pub field_type: String,
}

#[must_use]
pub fn index_body() -> IndexBody {
    IndexBody {
        settings: IndexSettings {
            index: IndexKnobs {
                knn: true,
                ef_search: EF_SEARCH,
            },
        },
        mappings: IndexMappings {
            properties: IndexProperties {
                vector: VectorField {
                    field_type: VECTOR_TYPE.to_owned(),
                    dimension: VECTOR_DIMENSION,
                    method: VectorMethod {
                        engine: "faiss".to_owned(),
                        name: "hnsw".to_owned(),
                        space_type: "l2".to_owned(),
                    },
                },
                metadata: MetadataField {
                    field_type: TEXT_TYPE.to_owned(),
                    index: false,
                },
                text_chunk: TextField {
                    field_type: TEXT_TYPE.to_owned(),
                },
            },
        },
    }
}

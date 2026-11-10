use super::presence::Field;
use super::presence::optional;
use serde::{Deserialize, Serialize};

use super::SapModelError;
use super::presence::{Given, NonEmpty};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(try_from = "i64")]
pub struct Count(i64);

impl TryFrom<i64> for Count {
    type Error = SapModelError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value < 0 {
            return Err(SapModelError::Rejected {
                message: String::from("the count must be zero or more"),
            });
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeyValueListPair {
    pub key: String,
    pub value: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum SelectMode {
    #[serde(rename = "ignoreIfKeyAbsent")]
    IgnoreIfKeyAbsent,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetadataPairs {
    pub key: String,
    pub value: Vec<String>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub select_mode: Field<Vec<SelectMode>>,
}

#[derive(Debug, Deserialize)]
struct GroundingSearchConfigRaw {
    #[serde(default, with = "optional")]
    max_chunk_count: Field<Count>,
    #[serde(default, with = "optional")]
    max_document_count: Field<Count>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(try_from = "GroundingSearchConfigRaw")]
pub struct GroundingSearchConfig {
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub max_chunk_count: Field<Count>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub max_document_count: Field<Count>,
}

impl TryFrom<GroundingSearchConfigRaw> for GroundingSearchConfig {
    type Error = SapModelError;

    fn try_from(raw: GroundingSearchConfigRaw) -> Result<Self, Self::Error> {
        let chunk_count = matches!(&raw.max_chunk_count, Some(Some(_)));
        let document_count = matches!(&raw.max_document_count, Some(Some(_)));
        if chunk_count && document_count {
            return Err(SapModelError::Rejected {
                message: String::from("Cannot specify both maxChunkCount and maxDocumentCount."),
            });
        }
        Ok(Self {
            max_chunk_count: raw.max_chunk_count,
            max_document_count: raw.max_document_count,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum DataRepositoryType {
    #[serde(rename = "vector")]
    Vector,
    #[serde(rename = "help.sap.com")]
    HelpSapCom,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DocumentGroundingFilter {
    #[serde(
        rename = "id",
        default,
        with = "optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub id: Field<String>,
    pub data_repository_type: DataRepositoryType,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub search_config: Field<GroundingSearchConfig>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub data_repositories: Field<Vec<String>>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub data_repository_metadata: Field<Vec<KeyValueListPair>>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub document_metadata: Field<Vec<MetadataPairs>>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub chunk_metadata: Field<Vec<KeyValueListPair>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DocumentGroundingPlaceholders {
    pub input: NonEmpty<String>,
    pub output: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DocumentGroundingConfig {
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub filters: Field<Vec<DocumentGroundingFilter>>,
    pub placeholders: DocumentGroundingPlaceholders,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub metadata_params: Field<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundingKind {
    DocumentGroundingService,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GroundingModuleConfig {
    #[serde(rename = "type", default, skip_serializing_if = "Given::is_missing")]
    pub type_: Given<GroundingKind>,
    pub config: DocumentGroundingConfig,
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KnowledgeBaseType {
    Vector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StorageType {
    OpensearchServerless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DataSourceType {
    S3,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateKnowledgeBaseRequest {
    pub client_token: String,
    pub name: String,
    pub role_arn: String,
    pub knowledge_base_configuration: KnowledgeBaseConfiguration,
    pub storage_configuration: StorageConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBaseConfiguration {
    #[serde(rename = "type")]
    pub knowledge_base_type: KnowledgeBaseType,
    #[serde(rename = "vectorKnowledgeBaseConfiguration")]
    pub vector: VectorConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorConfiguration {
    pub embedding_model_arn: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageConfiguration {
    #[serde(rename = "type")]
    pub storage_type: StorageType,
    #[serde(rename = "opensearchServerlessConfiguration")]
    pub aoss: AossConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AossConfiguration {
    pub collection_arn: String,
    pub field_mapping: AossFieldMapping,
    pub vector_index_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AossFieldMapping {
    pub metadata_field: String,
    pub text_field: String,
    pub vector_field: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateKnowledgeBaseResponse {
    pub knowledge_base: CreatedKnowledgeBase,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedKnowledgeBase {
    pub knowledge_base_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetKnowledgeBaseResponse {
    pub knowledge_base: KnowledgeBaseState,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBaseState {
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDataSourcesResponse {
    #[serde(default)]
    pub data_source_summaries: Vec<DataSourceSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceSummary {
    pub data_source_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDataSourceResponse {
    pub data_source: DataSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSource {
    #[serde(default)]
    pub data_source_configuration: Option<DataSourceConfiguration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceConfiguration {
    #[serde(default)]
    pub s3_configuration: Option<S3DataSourceConfiguration>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3DataSourceConfiguration {
    #[serde(default)]
    pub bucket_arn: Option<String>,
    #[serde(default)]
    pub inclusion_prefixes: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDataSourceRequest {
    pub client_token: String,
    pub name: String,
    pub data_source_configuration: NewDataSourceConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewDataSourceConfiguration {
    #[serde(rename = "type")]
    pub data_source_type: DataSourceType,
    pub s3_configuration: NewS3DataSourceConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewS3DataSourceConfiguration {
    pub bucket_arn: String,
    pub inclusion_prefixes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDataSourceResponse {
    pub data_source: CreatedDataSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedDataSource {
    pub data_source_id: String,
}

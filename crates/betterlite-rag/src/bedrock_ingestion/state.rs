#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreatedResources {
    pub s3_bucket: Option<String>,
    pub opensearch_collection: Option<String>,
    pub iam_role: Option<String>,
    pub knowledge_base: Option<String>,
    pub data_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockRagState {
    pub data_source_id: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_prefix: String,
    pub created_resources: CreatedResources,
}

pub mod observability;
mod pricing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestBanCatalog {
    pub banned: Vec<&'static str>,
    pub nested_config: [&'static str; 2],
    pub metadata: [&'static str; 2],
}

const BASE_FIELDS: [&str; 28] = [
    "api_base",
    "base_url",
    "user_config",
    "aws_sts_endpoint",
    "aws_web_identity_token",
    "aws_role_name",
    "aws_profile_name",
    "aws_session_name",
    "aws_external_id",
    "aws_session_tags",
    "vertex_credentials",
    "azure_ad_token",
    "aws_bedrock_runtime_endpoint",
    "aws_bedrock_project_id",
    "workspace_id",
    "aws_workspace_id",
    "anthropic_workspace_id",
    "anthropic-workspace-id",
    "bedrock_tags",
    "agentcore_runtime_session_id",
    "s3_endpoint_url",
    "sagemaker_base_url",
    "deployment_url",
    "nvcf_function_id",
    "use_ssl",
    "rust",
    "model_list",
    "vertex_ai_credentials",
];

impl RequestBanCatalog {
    #[must_use]
    pub fn pinned() -> Self {
        let mut banned = BASE_FIELDS.to_vec();
        banned.extend(observability::banned_fields());
        let mut pricing = [pricing::MIRRORED_FIELDS, pricing::CUSTOM_FIELDS].concat();
        pricing.sort_unstable();
        banned.extend(pricing);
        Self {
            banned,
            nested_config: ["litellm_embedding_config", "extra_body"],
            metadata: ["metadata", "litellm_metadata"],
        }
    }
}

mod tests;

use super::ParameterMap;
use super::provider_config::ResponsesApiConfig;
use betterlite_context::non_default_params::names::{
    DEFAULT_CHAT_COMPLETION_PARAMS, OPENAI_CHAT_COMPLETION_PARAMS,
};
use betterlite_context::owned_parameter_names::owned_names;
pub const NEVER_BRIDGED: &str = "client_metadata";
const GENERIC_PARAM_FIELDS: &str = "aws_access_key_id aws_bedrock_project_id aws_bedrock_runtime_endpoint aws_external_id aws_profile_name aws_region_name aws_role_name aws_secret_access_key aws_session_name aws_session_tags aws_session_token aws_sts_endpoint aws_web_identity_token azure_ad_token bucket_name drop_params milvus_db_name milvus_partition_names milvus_text_field organization timeout valkey_embedding_field valkey_host valkey_password valkey_port valkey_ssl valkey_text_field vector_store_id vertex_credentials vertex_location vertex_project watsonx_region_name";
#[must_use]
pub fn bridge_kwargs(
    kwargs: &ParameterMap,
    config: Option<&ResponsesApiConfig>,
    allowed_openai_params: &[String],
) -> ParameterMap {
    let forwarded = config.map(|_resolved| forwarded_names(allowed_openai_params));
    let mut kept = ParameterMap::new();
    for (name, value) in kwargs {
        let keep = match &forwarded {
            None => name != NEVER_BRIDGED,
            Some(names) => names.iter().any(|known| known == name),
        };
        if keep {
            kept.insert(name.clone(), value.clone());
        }
    }
    kept
}
fn forwarded_names(allowed_openai_params: &[String]) -> Vec<String> {
    let mut names: Vec<String> = OPENAI_CHAT_COMPLETION_PARAMS
        .split_whitespace()
        .chain(DEFAULT_CHAT_COMPLETION_PARAMS.split_whitespace())
        .chain(GENERIC_PARAM_FIELDS.split_whitespace())
        .map(str::to_owned)
        .collect();
    names.extend(owned_names());
    names.extend(allowed_openai_params.iter().cloned());
    names.retain(|name| name != NEVER_BRIDGED);
    names
}

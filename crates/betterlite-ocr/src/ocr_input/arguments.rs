use crate::ocr_error::OcrError;
use crate::ocr_provider::OcrConfigKind;
use crate::ocr_provider::resolve::resolve_provider_config;

const COMMON_OPTION_FIELDS: [&str; 3] = ["req_format", "extra_body", "max_response_bytes"];
const AZURE_AUTH_OPTION_FIELDS: [&str; 9] = [
    "azure_ad_token",
    "tenant_id",
    "client_id",
    "client_secret",
    "azure_scope",
    "azure_authority_host",
    "azure_credential",
    "azure_federated_token_file",
    "enable_azure_ad_token_refresh",
];
const AWS_AUTH_OPTION_FIELDS: [&str; 10] = [
    "aws_access_key_id",
    "aws_secret_access_key",
    "aws_session_token",
    "aws_region_name",
    "aws_session_name",
    "aws_profile_name",
    "aws_role_name",
    "aws_web_identity_token",
    "aws_sts_endpoint",
    "aws_external_id",
];
const VERTEX_AUTH_OPTION_FIELDS: [&str; 6] = [
    "vertex_credentials",
    "vertex_ai_credentials",
    "vertex_project",
    "vertex_ai_project",
    "vertex_location",
    "vertex_ai_location",
];
// wire.rs adds these to every Vertex DeepSeek call on top of the provider's own options
const VERTEX_DEEPSEEK_OPTION_FIELDS: [&str; 6] =
    ["stream", "temperature", "max_tokens", "top_p", "n", "stop"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgumentSpec {
    pub name: &'static str,
    pub secret: bool,
}

#[must_use]
pub fn is_supported_request(model: &str, custom_llm_provider: Option<&str>) -> bool {
    match resolve_provider_config(model, custom_llm_provider) {
        Ok((_resolved_model, _config)) => true,
        Err(_error) => false,
    }
}

pub fn consumed_optional_param_names(
    model: &str,
    custom_llm_provider: Option<&str>,
) -> Result<Vec<&'static str>, OcrError> {
    let (resolved_model, config) = resolve_provider_config(model, custom_llm_provider)?;
    let auth_fields: &[&str] = match config {
        OcrConfigKind::AwsTextract | OcrConfigKind::AwsTextractAnalyze => &AWS_AUTH_OPTION_FIELDS,
        OcrConfigKind::AzureAi
        | OcrConfigKind::AzureCohere
        | OcrConfigKind::AzureDocumentIntelligence => &AZURE_AUTH_OPTION_FIELDS,
        OcrConfigKind::VertexAi | OcrConfigKind::VertexDeepSeek => &VERTEX_AUTH_OPTION_FIELDS,
        OcrConfigKind::Cohere
        | OcrConfigKind::Mistral
        | OcrConfigKind::ReductoLegacy
        | OcrConfigKind::ReductoV3 => &[],
    };
    let mut names: Vec<&'static str> = COMMON_OPTION_FIELDS
        .iter()
        .chain(config.get_supported_ocr_params(&resolved_model))
        .chain(auth_fields)
        .copied()
        .collect();
    if config == OcrConfigKind::VertexDeepSeek {
        names.extend(VERTEX_DEEPSEEK_OPTION_FIELDS);
    }
    Ok(names)
}

pub fn consumed_optional_params(
    model: &str,
    custom_llm_provider: Option<&str>,
) -> Result<Vec<ArgumentSpec>, OcrError> {
    let names = consumed_optional_param_names(model, custom_llm_provider)?;
    Ok(names
        .into_iter()
        .map(|name| ArgumentSpec {
            name,
            secret: is_secret_param(name),
        })
        .collect())
}

#[must_use]
pub fn is_secret_param(name: &str) -> bool {
    matches!(
        name,
        "azure_ad_token"
            | "client_secret"
            | "azure_federated_token_file"
            | "vertex_credentials"
            | "vertex_ai_credentials"
            | "aws_secret_access_key"
            | "aws_session_token"
            | "aws_web_identity_token"
    )
}

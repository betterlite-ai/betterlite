use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// _CONFIG_CONNECTION_FIELDS: the admin clear-on-override fields, clientside_credential_keys and litellm_credential_name
pub const CONFIG_CONNECTION_FIELDS: [&str; 56] = [
    "api_base",
    "api_key",
    "api_type",
    "api_version",
    "aws_access_key_id",
    "aws_batch_role_arn",
    "aws_bedrock_project_id",
    "aws_bedrock_runtime_endpoint",
    "aws_external_id",
    "aws_profile_name",
    "aws_region_name",
    "aws_role_name",
    "aws_secret_access_key",
    "aws_session_name",
    "aws_session_tags",
    "aws_session_token",
    "aws_sts_endpoint",
    "aws_web_identity_token",
    "azure_ad_token",
    "azure_ad_token_provider",
    "azure_password",
    "azure_scope",
    "azure_username",
    "base_url",
    "bedrock_tags",
    "bucket_name",
    "client_id",
    "client_secret",
    "default_headers",
    "extra_body",
    "extra_headers",
    "gcs_bucket_name",
    "litellm_credential_name",
    "nvcf_function_id",
    "oci_fingerprint",
    "oci_key",
    "oci_key_file",
    "oci_signer",
    "oci_tenancy",
    "oci_user",
    "organization",
    "region_name",
    "s3_access_key_id",
    "s3_bucket_name",
    "s3_bucket_owner",
    "s3_encryption_key_id",
    "s3_endpoint_url",
    "s3_output_bucket_name",
    "s3_region_name",
    "s3_secret_access_key",
    "tenant_id",
    "use_ssl",
    "vertex_credentials",
    "vertex_location",
    "vertex_project",
    "watsonx_region_name",
];

// The 400 a request-supplied os.environ/ reference earns; config values were resolved before they reached here.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("Environment variable references are not permitted in request parameters.")]
pub struct EnvironmentReference;

// _reject_os_environ_references: a depth-first scan of values only, since a dict's keys are never resolved.
pub fn reject_os_environ_references(params: &JsonObject) -> Result<(), EnvironmentReference> {
    if params.values().any(has_environment_reference) {
        return Err(EnvironmentReference);
    }
    Ok(())
}

fn has_environment_reference(value: &Json) -> bool {
    match value {
        Json::Text(text) => text.starts_with("os.environ/"),
        Json::List(items) => items.iter().any(has_environment_reference),
        Json::Object(map) => map.values().any(has_environment_reference),
        Json::Null | Json::Bool(_) | Json::Number(_) => false,
    }
}

// _request_inherits_config_credentials: a request naming a stored credential of its own does not borrow the config's; a blank name is no name.
#[must_use]
pub fn inherits_config_credentials(
    config_params: &JsonObject,
    request_params: &JsonObject,
    allow_client_side_credentials: bool,
    banned: &[&str],
) -> bool {
    let requested = request_params.get("litellm_credential_name");
    let names_credential = matches!(requested, Some(Json::Text(name)) if !name.is_empty());
    if names_credential && requested != config_params.get("litellm_credential_name") {
        return false;
    }
    if allow_client_side_credentials {
        return true;
    }
    !banned
        .iter()
        .any(|param| request_params.contains_key(*param))
}

// _config_base_for_health_check: the config's own merged credentials are dropped once the request has its own.
#[must_use]
pub fn config_base_for_health_check(
    config_params: &JsonObject,
    request_params: &JsonObject,
    allow_client_side_credentials: bool,
    banned: &[&str],
) -> JsonObject {
    if inherits_config_credentials(
        config_params,
        request_params,
        allow_client_side_credentials,
        banned,
    ) {
        return config_params.clone();
    }
    config_params
        .iter()
        .filter(|(key, _value)| !CONFIG_CONNECTION_FIELDS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

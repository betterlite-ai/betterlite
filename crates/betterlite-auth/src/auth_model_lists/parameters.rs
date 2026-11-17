use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
// CredentialLiteLLMParams.model_fields, in declared order; the same table as betterlite-router's deployment_clients::lookup.
#[rustfmt::skip]
pub const CREDENTIAL_FIELDS: [&str; 40] = [
    "api_key", "api_base", "api_version", "azure_ad_token", "tenant_id", "client_id",
    "client_secret", "azure_scope", "azure_username", "azure_password", "vertex_project",
    "vertex_location", "vertex_credentials", "region_name", "gcs_bucket_name", "bucket_name",
    "aws_access_key_id", "aws_secret_access_key", "aws_session_token", "aws_region_name",
    "aws_session_name", "aws_profile_name", "aws_role_name", "aws_web_identity_token",
    "aws_sts_endpoint", "aws_external_id", "aws_session_tags", "aws_bedrock_runtime_endpoint",
    "aws_bedrock_project_id", "s3_bucket_name", "s3_endpoint_url", "s3_region_name",
    "s3_access_key_id", "s3_secret_access_key", "s3_encryption_key_id", "s3_bucket_owner",
    "aws_batch_role_arn", "s3_output_bucket_name", "bedrock_tags", "watsonx_region_name",
];
// pydantic declares vertex_credentials str | dict and the two tag fields a list, never text.
const TEXT_OR_OBJECT_FIELDS: [&str; 1] = ["vertex_credentials"];
const LIST_ONLY_FIELDS: [&str; 2] = ["aws_session_tags", "bedrock_tags"];
const MODEL_FIELD: &str = "model";
const CREDENTIAL_NAME_FIELD: &str = "litellm_credential_name";
#[derive(Debug, thiserror::Error)]
pub enum ParameterError {
    #[error("litellm_params.model is required and must be text")]
    Model,
    #[error("litellm_params.{field} does not have the type LiteLLM declares for it")]
    Field { field: &'static str },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelParameters {
    model: String,
    fields: JsonObject,
}
impl ModelParameters {
    pub fn parse(fields: &JsonObject) -> Result<Self, ParameterError> {
        let Some(Json::Text(model)) = fields.get(MODEL_FIELD) else {
            return Err(ParameterError::Model);
        };
        for field in CREDENTIAL_FIELDS {
            let Some(value) = fields.get(field) else {
                continue;
            };
            if allowed_credential_value(field, value) {
                continue;
            }
            return Err(ParameterError::Field { field });
        }
        if let Some(value) = fields.get(CREDENTIAL_NAME_FIELD)
            && !matches!(value, Json::Null | Json::Text(_))
        {
            return Err(ParameterError::Field {
                field: CREDENTIAL_NAME_FIELD,
            });
        }
        Ok(Self {
            model: model.clone(),
            fields: fields.clone(),
        })
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn model(&self) -> &str {
        &self.model
    }

    #[must_use]
    pub fn text(&self, field: &str) -> Option<&str> {
        match self.fields.get(field) {
            Some(Json::Text(value)) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn credential_name(&self) -> Option<&str> {
        self.text(CREDENTIAL_NAME_FIELD)
    }

    #[must_use]
    pub fn is_null(&self, field: &str) -> bool {
        self.fields
            .get(field)
            .is_none_or(|value| *value == Json::Null)
    }

    pub fn fill_null(&mut self, field: &str, value: &Json) {
        if self.is_null(field) {
            self.fields.insert(field.to_owned(), value.clone());
        }
    }

    pub fn clear_credential_name(&mut self) {
        self.fields
            .insert(CREDENTIAL_NAME_FIELD.to_owned(), Json::Null);
    }
}
fn allowed_credential_value(field: &str, value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Text(_) => !LIST_ONLY_FIELDS.contains(&field),
        Json::Object(_) => TEXT_OR_OBJECT_FIELDS.contains(&field),
        Json::List(_) => LIST_ONLY_FIELDS.contains(&field),
        Json::Bool(_) | Json::Number(_) => false,
    }
}

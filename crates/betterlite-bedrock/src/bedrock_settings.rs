use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RawValue {
    Null(()),
    Bool(bool),
    Number(f64),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AwsInput {
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<String>,
    pub aws_session_token: Option<String>,
    pub aws_region_name: Option<String>,
    pub aws_session_name: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_web_identity_token: Option<String>,
    pub aws_sts_endpoint: Option<String>,
    pub aws_external_id: Option<String>,
    pub aws_role_arn: Option<String>,
    pub aws_web_identity_token_file: Option<String>,
    pub aws_bearer_token_bedrock: Option<String>,
    pub aws_session_tags: Option<RawValue>,
}

#[derive(Debug, Clone, Default)]
pub struct AwsSettings {
    pub aws_access_key_id: Option<SecretString>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
    pub aws_region_name: Option<String>,
    pub aws_session_name: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>,
    pub aws_external_id: Option<SecretString>,
    pub aws_role_arn: Option<String>,
    pub aws_web_identity_token_file: Option<String>,
    pub aws_bearer_token_bedrock: Option<SecretString>,
    pub aws_session_tags: Option<RawValue>,
}

pub trait Environment {
    fn get(&self, name: &str) -> Option<String>;
}
impl Environment for BTreeMap<String, String> {
    fn get(&self, name: &str) -> Option<String> {
        self.get(name).cloned()
    }
}

pub fn snapshot<E>(deployment: &AwsInput, module: &AwsInput, environment: &E) -> AwsSettings
where
    E: Environment,
{
    let text = |deployment_value: &Option<String>, module_value: &Option<String>, name: &str| {
        deployment_value
            .clone()
            .or_else(|| module_value.clone())
            .or_else(|| environment.get(name))
    };
    let secret = |deployment_value: &Option<String>, module_value: &Option<String>, name: &str| {
        text(deployment_value, module_value, name).map(SecretString::from)
    };
    let tags = deployment.aws_session_tags.clone();
    macro_rules! select {
        ($kind:ident, $field:ident, $name:literal) => {
            $kind(&deployment.$field, &module.$field, $name)
        };
    }
    AwsSettings {
        aws_access_key_id: select!(secret, aws_access_key_id, "AWS_ACCESS_KEY_ID"),
        aws_secret_access_key: select!(secret, aws_secret_access_key, "AWS_SECRET_ACCESS_KEY"),
        aws_session_token: select!(secret, aws_session_token, "AWS_SESSION_TOKEN"),
        aws_region_name: select!(text, aws_region_name, "AWS_REGION_NAME"),
        aws_session_name: select!(text, aws_session_name, "AWS_SESSION_NAME"),
        aws_profile_name: select!(text, aws_profile_name, "AWS_PROFILE_NAME"),
        aws_role_name: select!(text, aws_role_name, "AWS_ROLE_NAME"),
        aws_web_identity_token: select!(secret, aws_web_identity_token, "AWS_WEB_IDENTITY_TOKEN"),
        aws_sts_endpoint: select!(text, aws_sts_endpoint, "AWS_STS_ENDPOINT"),
        aws_external_id: select!(secret, aws_external_id, "AWS_EXTERNAL_ID"),
        aws_role_arn: select!(text, aws_role_arn, "AWS_ROLE_ARN"),
        aws_web_identity_token_file: select!(
            text,
            aws_web_identity_token_file,
            "AWS_WEB_IDENTITY_TOKEN_FILE"
        ),
        aws_bearer_token_bedrock: select!(
            secret,
            aws_bearer_token_bedrock,
            "AWS_BEARER_TOKEN_BEDROCK"
        ),
        aws_session_tags: tags.or_else(|| module.aws_session_tags.clone()),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub request: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct BedrockError {
    pub status: u16,
    pub message: String,
    pub headers: Option<BTreeMap<String, String>>,
    pub response: Option<ErrorResponse>,
}

#[must_use]
pub fn synthesize_error_response(
    status: u16,
    headers: Option<&BTreeMap<String, RawValue>>,
    request: Option<String>,
) -> Option<ErrorResponse> {
    let provided_headers = headers.filter(|provided| !provided.is_empty())?;
    let filtered = string_headers(
        provided_headers
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    Some(ErrorResponse {
        status,
        headers: filtered,
        request: request.unwrap_or_else(|| "POST https://docs.litellm.ai/docs".to_owned()),
    })
}

#[must_use]
pub fn bedrock_error(
    status: u16,
    message: String,
    headers: Option<BTreeMap<String, RawValue>>,
    request: Option<String>,
) -> BedrockError {
    let response = synthesize_error_response(status, headers.as_ref(), request);
    let filtered_headers = headers.map(string_headers);
    BedrockError {
        status,
        message,
        headers: filtered_headers,
        response,
    }
}

fn string_headers<I>(values: I) -> BTreeMap<String, String>
where
    I: IntoIterator<Item = (String, RawValue)>,
{
    let mut filtered = BTreeMap::new();
    for (key, value) in values {
        if let RawValue::Text(text) = value {
            filtered.insert(key, text);
        }
    }
    filtered
}

mod tests;

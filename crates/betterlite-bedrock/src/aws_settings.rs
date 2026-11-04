use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod credential_cache;
mod sources;

pub use credential_cache::{CachedCredentials, cache_key, get_or_fetch};
pub use sources::{
    LocalCredentialInputs, LocalCredentialProvider, ResolvedCredentials, auth_selector_presence,
    resolve_local_credentials,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwsSessionTag {
    #[serde(rename = "Key")]
    pub key: String,
    #[serde(rename = "Value")]
    pub value: String,
}

pub fn canonical_session_tags(tags: Option<Vec<AwsSessionTag>>) -> Option<Vec<AwsSessionTag>> {
    tags.map(|mut tags| {
        tags.sort_by(|left, right| left.key.cmp(&right.key));
        tags
    })
}

pub fn s3_static_key_pair(parameters: &Value) -> Option<(String, String)> {
    let access = parameters.get("s3_access_key_id")?.as_str()?;
    let secret = parameters.get("s3_secret_access_key")?.as_str()?;
    if access.is_empty() || secret.is_empty() {
        return None;
    }
    Some((access.to_owned(), secret.to_owned()))
}

pub fn bedrock_bearer_token(
    api_key: Option<&str>,
    configured_token: Option<&SecretString>,
) -> Option<SecretString> {
    let selected = api_key.map(str::to_owned).or_else(|| {
        configured_token.map(|token| {
            use secrecy::ExposeSecret as _;
            token.expose_secret().to_owned()
        })
    })?;
    if selected.is_empty() {
        None
    } else {
        Some(SecretString::from(selected))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid aws_session_tags: expected string Key and Value fields")]
    SessionTags,
}

pub fn canonicalize_tags(
    tags: Option<Vec<Value>>,
) -> Result<Option<Vec<AwsSessionTag>>, AuthError> {
    tags.map(|values| {
        let mut tags = values
            .into_iter()
            .map(|value| {
                let key = value.get("Key").and_then(Value::as_str);
                let value_text = value.get("Value").and_then(Value::as_str);
                match (key, value_text) {
                    (Some(key), Some(value)) => Ok(AwsSessionTag {
                        key: key.to_owned(),
                        value: value.to_owned(),
                    }),
                    _ => Err(AuthError::SessionTags),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        tags.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(tags)
    })
    .transpose()
}

pub fn assume_role_parameters(
    role_arn: String,
    role_session_name: String,
    external_id: Option<String>,
    tags: Option<Vec<AwsSessionTag>>,
) -> Value {
    let mut parameters = serde_json::Map::from_iter([
        ("RoleArn".to_owned(), Value::String(role_arn)),
        (
            "RoleSessionName".to_owned(),
            Value::String(role_session_name),
        ),
    ]);
    if let Some(external_id) = external_id {
        parameters.insert("ExternalId".to_owned(), Value::String(external_id));
    }
    if let Some(tags) = tags.filter(|tags| !tags.is_empty()) {
        parameters.insert("Tags".to_owned(), json!(tags));
    }
    Value::Object(parameters)
}

pub fn web_identity_policy() -> Value {
    let statements = [
        (
            "BedrockLiteLLM",
            vec![
                "bedrock:InvokeModel",
                "bedrock:InvokeModelWithResponseStream",
                "bedrock:CountTokens",
                "bedrock:Rerank",
                "bedrock:Retrieve",
                "bedrock:ListKnowledgeBases",
                "bedrock:InvokeAgent",
                "bedrock:ApplyGuardrail",
                "bedrock:GetGuardrail",
                "bedrock:ListGuardrails",
            ],
        ),
        (
            "BedrockAgentCoreLiteLLM",
            vec![
                "bedrock-agentcore:InvokeAgentRuntime",
                "bedrock-agentcore:InvokeAgentRuntimeForUser",
                "bedrock-agentcore:InvokeGateway",
            ],
        ),
        (
            "ClaudePlatformLiteLLM",
            vec![
                "aws-external-anthropic:CreateInference",
                "aws-external-anthropic:CreateBatchInference",
                "aws-external-anthropic:CancelBatchInference",
                "aws-external-anthropic:DeleteBatchInference",
                "aws-external-anthropic:CountTokens",
                "aws-external-anthropic:Get*",
                "aws-external-anthropic:List*",
            ],
        ),
        (
            "BedrockMantleLiteLLM",
            vec!["bedrock-mantle:CreateInference"],
        ),
    ];
    let statements = statements
        .into_iter()
        .map(|(sid, actions)| {
            json!({
                "Sid": sid,
                "Effect": "Allow",
                "Action": actions,
                "Resource": "*",
                "Condition": {"Bool": {"aws:SecureTransport": "true"}}
            })
        })
        .collect::<Vec<_>>();
    json!({"Version": "2012-10-17", "Statement": statements})
}

pub fn pop_aws_auth_params(parameters: &mut Value) -> serde_json::Map<String, Value> {
    let mut popped = serde_json::Map::new();
    if let Some(object) = parameters.as_object_mut() {
        for key in [
            "aws_access_key_id",
            "aws_secret_access_key",
            "aws_session_token",
            "aws_session_name",
            "aws_profile_name",
            "aws_role_name",
            "aws_web_identity_token",
            "aws_sts_endpoint",
            "aws_external_id",
            "aws_session_tags",
        ] {
            popped.insert(key.to_owned(), object.remove(key).unwrap_or(Value::Null));
        }
    }
    popped
}

#[derive(Debug, serde::Serialize)]
pub struct AwsAuthError {
    pub status: u16,
    pub message: String,
    pub request: betterlite_openai::error::ErrorRequest,
    pub response: betterlite_openai::error::ErrorResponse,
}

pub fn aws_auth_error_projection(status: u16, message: String) -> AwsAuthError {
    let request = betterlite_openai::error::ErrorRequest {
        method: "POST".to_owned(),
        url: "https://us-west-2.console.aws.amazon.com/bedrock".to_owned(),
    };
    AwsAuthError {
        status,
        message,
        response: betterlite_openai::error::ErrorResponse {
            status_code: status,
            request: request.clone(),
        },
        request,
    }
}

mod tests;

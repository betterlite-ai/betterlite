use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::Helper;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    deployment: serde_json::Map<String, Value>,
    #[serde(default)]
    litellm_settings: serde_json::Map<String, Value>,
    #[serde(default)]
    env: serde_json::Map<String, Value>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let names = [
            ("aws_access_key_id", "AWS_ACCESS_KEY_ID"),
            ("aws_secret_access_key", "AWS_SECRET_ACCESS_KEY"),
            ("aws_session_token", "AWS_SESSION_TOKEN"),
            ("aws_region_name", "AWS_REGION_NAME"),
            ("aws_session_name", "AWS_SESSION_NAME"),
            ("aws_profile_name", "AWS_PROFILE_NAME"),
            ("aws_role_name", "AWS_ROLE_NAME"),
            ("aws_web_identity_token", "AWS_WEB_IDENTITY_TOKEN"),
            ("aws_sts_endpoint", "AWS_STS_ENDPOINT"),
            ("aws_external_id", "AWS_EXTERNAL_ID"),
            ("aws_role_arn", "AWS_ROLE_ARN"),
            ("aws_web_identity_token_file", "AWS_WEB_IDENTITY_TOKEN_FILE"),
            ("aws_bearer_token_bedrock", "AWS_BEARER_TOKEN_BEDROCK"),
        ];
        let mut result = serde_json::Map::new();
        for (name, environment) in names {
            let value = input
                .deployment
                .get(name)
                .or_else(|| input.litellm_settings.get(name))
                .or_else(|| input.env.get(environment))
                .cloned()
                .unwrap_or(Value::Null);
            result.insert(name.to_owned(), value);
        }
        result.insert(
            "aws_session_tags".to_owned(),
            input
                .deployment
                .get("aws_session_tags")
                .or_else(|| input.litellm_settings.get("aws_session_tags"))
                .cloned()
                .unwrap_or(Value::Null),
        );
        Ok(json!(result))
    }
}

mod tests;

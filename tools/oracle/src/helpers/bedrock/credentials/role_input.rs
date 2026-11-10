use std::collections::BTreeMap;

use betterlite_bedrock::aws_settings::AwsSessionTag;
use betterlite_bedrock::credentials::role::RoleAuth;
use secrecy::SecretString;
use serde_json::Value;

use super::input::Step;
use super::ops::failure;
use crate::helpers::HelperError;

pub(super) struct OwnedRoleAuth {
    role_name: String,
    session_name: String,
    access_key_id: Option<SecretString>,
    secret_access_key: Option<SecretString>,
    session_token: Option<SecretString>,
    external_id: Option<String>,
    irsa_role_arn: Option<String>,
    web_identity_token_file: Option<String>,
    tags: Option<Vec<AwsSessionTag>>,
}

impl OwnedRoleAuth {
    pub(super) fn parse(
        step: &Step,
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, Box<HelperError>> {
        let tags = step
            .params
            .get("aws_session_tags")
            .map(|value| serde_json::from_value::<Vec<AwsSessionTag>>(value.clone()))
            .transpose()
            .map_err(failure)?;
        Ok(Self {
            role_name: step
                .params
                .get("aws_role_name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_default(),
            session_name: step
                .params
                .get("aws_session_name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_default(),
            access_key_id: step
                .params
                .get("aws_access_key_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .map(SecretString::from),
            secret_access_key: step
                .params
                .get("aws_secret_access_key")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .map(SecretString::from),
            session_token: step
                .params
                .get("aws_session_token")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .map(SecretString::from),
            external_id: step
                .params
                .get("aws_external_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            irsa_role_arn: environment.get("AWS_ROLE_ARN").cloned(),
            web_identity_token_file: environment.get("AWS_WEB_IDENTITY_TOKEN_FILE").cloned(),
            tags,
        })
    }

    pub(super) fn auth(&self) -> RoleAuth<'_> {
        RoleAuth {
            role_name: &self.role_name,
            session_name: &self.session_name,
            access_key_id: self.access_key_id.as_ref(),
            secret_access_key: self.secret_access_key.as_ref(),
            session_token: self.session_token.as_ref(),
            external_id: self.external_id.as_deref(),
            irsa_role_arn: self.irsa_role_arn.as_deref(),
            web_identity_token_file: self.web_identity_token_file.as_deref(),
            tags: self.tags.as_deref(),
        }
    }
}

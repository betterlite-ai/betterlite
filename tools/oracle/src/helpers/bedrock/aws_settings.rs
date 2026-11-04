use betterlite_bedrock::aws_settings::{
    assume_role_parameters, aws_auth_error_projection, bedrock_bearer_token, cache_key,
    canonicalize_tags, pop_aws_auth_params, s3_static_key_pair, web_identity_policy,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: String,
    params: Value,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input.operation.as_str() {
            "static_key_pair" => {
                let pair = s3_static_key_pair(&input.params)
                    .map(|(access, secret)| json!([access, secret]));
                json!({"value": pair})
            }
            "canonical_tags" => {
                let tags = input.params.get("tags").and_then(Value::as_array).cloned();
                let tags = canonicalize_tags(tags)
                    .map_err(|message| helper_error("ValueError", message.to_string()))?;
                json!({"value": tags})
            }
            "assume_role" => {
                let role = input
                    .params
                    .get("role_arn")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let session = input
                    .params
                    .get("session_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let external = input
                    .params
                    .get("external_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let tags = input.params.get("tags").and_then(Value::as_array).cloned();
                let tags = canonicalize_tags(tags)
                    .map_err(|message| helper_error("ValueError", message.to_string()))?;
                json!({"value": assume_role_parameters(role.to_owned(), session.to_owned(), external, tags)})
            }
            "web_policy" => json!({"value": web_identity_policy()}),
            "cache_key" => {
                let value = cache_key(&input.params)
                    .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
                json!({"value": value})
            }
            "auth_selector" => {
                let present =
                    |key: &str| input.params.get(key).is_some_and(|value| !value.is_null());
                let web_identity = present("aws_web_identity_token")
                    && present("aws_role_name")
                    && present("aws_session_name");
                let role = present("aws_role_name");
                let profile = present("aws_profile_name");
                let session = present("aws_access_key_id")
                    && present("aws_secret_access_key")
                    && present("aws_session_token");
                let static_pair = present("aws_access_key_id")
                    && present("aws_secret_access_key")
                    && present("aws_region_name");
                json!({"web_identity": web_identity, "role": role, "profile": profile, "session_token": session, "static": static_pair})
            }
            "session_auth" => {
                let access = input
                    .params
                    .get("aws_access_key_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let secret = input
                    .params
                    .get("aws_secret_access_key")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let token = input
                    .params
                    .get("aws_session_token")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                json!({"access_key_id": access, "secret_access_key": secret, "token": token})
            }
            "bearer" => {
                let api_key = input.params.get("api_key").and_then(Value::as_str);
                let configured = input
                    .params
                    .get("configured_token")
                    .and_then(Value::as_str)
                    .map(|value| SecretString::from(value.to_owned()));
                json!({"value": bedrock_bearer_token(api_key, configured.as_ref()).map(|value| value.expose_secret().to_owned())})
            }
            "pop_auth" => {
                let mut params = input.params.clone();
                let removed = pop_aws_auth_params(&mut params);
                json!({"remaining": params, "removed": removed})
            }
            "auth_error" => {
                let status = input
                    .params
                    .get("status")
                    .and_then(Value::as_u64)
                    .and_then(|value| u16::try_from(value).ok())
                    .unwrap_or_default();
                let message = input
                    .params
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let error = aws_auth_error_projection(status, message.to_owned());
                json!({"status": error.status, "message": error.message, "request": error.request, "response": {"status": error.response.status_code}})
            }
            _ => return Err(helper_error("InvalidInput", "unknown operation".to_owned())),
        };
        Ok(output)
    }
}

fn helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

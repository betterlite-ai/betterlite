use std::future::{Future, ready};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_vertex::aws_federation::{
    AwsCredentialsResolver, AwsParams, FederationInput, credentials_from_explicit_aws,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = FederationInput;

    fn run(input: &Self::Input) -> HelperResult {
        let retained = retained_fields(&input.aws);
        let region = match input.aws.region_name.as_deref() {
            Some(region) if !region.is_empty() => {
                let federation = credentials_from_explicit_aws(input.clone(), &UnusedResolver)
                    .or_value_error()?;
                Value::String(federation.supplier.region)
            }
            _ => Value::Null,
        };
        Ok(json!({"aws_params": retained, "aws_region": region}))
    }
}

fn retained_fields(params: &AwsParams) -> Value {
    let mut retained = Map::new();
    let secrets = [
        ("aws_access_key_id", params.access_key_id.as_ref()),
        ("aws_secret_access_key", params.secret_access_key.as_ref()),
        ("aws_session_token", params.session_token.as_ref()),
        ("aws_web_identity_token", params.web_identity_token.as_ref()),
    ];
    for (name, secret) in secrets {
        insert_secret(&mut retained, name, secret);
    }
    let texts = [
        ("aws_region_name", params.region_name.as_deref()),
        ("aws_session_name", params.session_name.as_deref()),
        ("aws_profile_name", params.profile_name.as_deref()),
        ("aws_role_name", params.role_name.as_deref()),
        ("aws_sts_endpoint", params.sts_endpoint.as_deref()),
        ("aws_external_id", params.external_id.as_deref()),
    ];
    for (name, text) in texts {
        insert_text(&mut retained, name, text);
    }
    Value::Object(retained)
}

fn insert_secret(retained: &mut Map<String, Value>, name: &str, secret: Option<&SecretString>) {
    if let Some(value) = secret {
        retained.insert(
            name.to_owned(),
            Value::String(value.expose_secret().to_owned()),
        );
    }
}

fn insert_text(retained: &mut Map<String, Value>, name: &str, text: Option<&str>) {
    if let Some(value) = text {
        retained.insert(name.to_owned(), Value::String(value.to_owned()));
    }
}

struct UnusedResolver;

impl AwsCredentialsResolver for UnusedResolver {
    fn credentials(
        &self,
        _settings: &AwsSettings,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        ready(Err(CredentialError::MissingCredentials))
    }
}

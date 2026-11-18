mod tests;

use std::collections::BTreeMap;

use betterlite_auth::sso_mappings::headers::decode_generic_headers;
use betterlite_auth::sso_mappings::roles::decode_role_mapping_literal;
use betterlite_auth::sso_mappings::settings::{MissingVariable, generic_sso_settings};
use betterlite_config::sso_mappings::{SsoMappingSources, load_mapping_sources};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    environment: BTreeMap<String, String>,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Call {
    RoleMappings,
    Headers,
    EnvVars,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let sources = load_mapping_sources(&input.environment);
        let answers = input
            .calls
            .iter()
            .map(|call| answer(&sources, call))
            .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
        Ok(json!({"calls": answers}))
    }
}

fn answer(sources: &SsoMappingSources, call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::RoleMappings => {
            let Some(literal) = sources.role_mappings_roles.as_deref() else {
                return Ok(json!({"role_mappings": null}));
            };
            let mappings = decode_role_mapping_literal(
                literal.as_bytes(),
                sources.role_mappings_group_claim.as_deref(),
                sources.role_mappings_default_role.as_deref(),
            )
            .or_value_error()?;
            Ok(json!({"role_mappings": mappings}))
        }
        Call::Headers => Ok(json!({
            "headers": decode_generic_headers(sources.sso_headers.as_deref().map(str::as_bytes))
                .or_value_error()?,
        })),
        Call::EnvVars => Ok(env_vars(sources)),
    }
}

fn env_vars(sources: &SsoMappingSources) -> Value {
    match generic_sso_settings(sources) {
        Ok(settings) => json!({"settings": {
            "client_secret": settings.client_secret.expose_secret(),
            "scope": settings.scope,
            "authorization_endpoint": settings.authorization_endpoint,
            "token_endpoint": settings.token_endpoint,
            "userinfo_endpoint": settings.userinfo_endpoint,
            "include_client_id": settings.include_client_id,
        }}),
        Err(MissingVariable(exception)) => json!({"error": {
            "type": exception.error_type,
            "param": exception.param,
            "code": exception.code,
            "message": exception.message,
        }}),
    }
}

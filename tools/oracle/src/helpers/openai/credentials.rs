use crate::helpers::{HelperError, HelperResult};
use betterlite_openai::credentials::{
    CredentialsError, ExplicitCredentials, LiteLlmGlobals, get_openai_credentials,
    infer_openai_data_residency, is_openai_backed_api_base, resolve_openai_api_base,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialFields {
    api_key: Option<String>,
    organization: Option<String>,
    litellm_api_key: Option<String>,
    litellm_openai_key: Option<String>,
    litellm_organization: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApiBaseFields {
    api_base: Option<String>,
    litellm_api_base: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostFields {
    api_base: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResidencyFields {
    custom_llm_provider: Option<String>,
    api_base: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CredentialsCall {
    KeyAndOrganizationPrecedence(CredentialFields),
    BaseUrlBeforeApiBase(ApiBaseFields),
    EmptyOrganization(CredentialFields),
    OpenaiHostBoundary(HostFields),
    RegionalDataResidency(ResidencyFields),
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn credentials(fields: &CredentialFields) -> Value {
    let explicit = ExplicitCredentials {
        api_base: None,
        api_key: fields.api_key.clone().map(SecretString::from),
        organization: fields.organization.clone(),
    };
    let globals = LiteLlmGlobals {
        api_base: None,
        api_key: fields.litellm_api_key.clone().map(SecretString::from),
        openai_key: fields.litellm_openai_key.clone().map(SecretString::from),
        organization: fields.litellm_organization.clone(),
    };
    let resolved = get_openai_credentials(explicit, globals, &fields.env);
    json!({
        "api_base": resolved.api_base,
        "api_key": resolved.api_key.as_ref().map(ExposeSecret::expose_secret),
        "organization": resolved.organization,
    })
}

fn base_url(fields: &ApiBaseFields) -> Value {
    let api_base = resolve_openai_api_base(
        fields.api_base.as_deref(),
        fields.litellm_api_base.as_deref(),
        &fields.env,
    );
    json!({ "api_base": api_base })
}

fn host_boundary(fields: &HostFields) -> HelperResult {
    is_openai_backed_api_base(&fields.api_base)
        .map(|accepted| json!({ "accepted": accepted }))
        .map_err(|error: CredentialsError| invalid_input(error.to_string()))
}

fn residency(fields: &ResidencyFields) -> HelperResult {
    let region = infer_openai_data_residency(
        fields.custom_llm_provider.as_deref(),
        fields.api_base.as_deref(),
    );
    serde_json::to_value(region).map_err(|error| invalid_input(error.to_string()))
}

fn invoke(input: &CredentialsCall) -> HelperResult {
    match input {
        CredentialsCall::KeyAndOrganizationPrecedence(fields)
        | CredentialsCall::EmptyOrganization(fields) => Ok(credentials(fields)),
        CredentialsCall::BaseUrlBeforeApiBase(fields) => Ok(base_url(fields)),
        CredentialsCall::OpenaiHostBoundary(fields) => host_boundary(fields),
        CredentialsCall::RegionalDataResidency(fields) => residency(fields),
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = CredentialsCall;

    fn run(input: &Self::Input) -> HelperResult {
        invoke(input)
    }
}

mod tests;

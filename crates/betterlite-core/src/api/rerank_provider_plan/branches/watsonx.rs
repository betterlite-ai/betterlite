use super::super::dispatch::{Headers, Params, provider_failure, value_failure};
use super::Credentials;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_error::dispatch::RawException;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolvedModel;
use betterlite_openai::watsonx_auth::credentials::get_watsonx_credentials;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::error::WatsonxError;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use secrecy::{ExposeSecret as _, SecretString};
pub(super) struct WatsonxRequest<'request> {
    pub(super) model: &'request str,
    pub(super) provider: &'request str,
    pub(super) resolved: &'request ResolvedModel,
    pub(super) generic_params: &'request Params,
    pub(super) environment: &'request dyn EnvironmentLookup,
    pub(super) headers: Headers,
}
// get_watsonx_credentials reads the whole generic map plus the resolved key and base and keeps a token that is not None
pub(super) fn watsonx_credentials(
    request: WatsonxRequest<'_>,
    mapped: &mut Params,
) -> Result<Credentials, RawException> {
    let mut params = WatsonxParams::new();
    for (name, value) in request.generic_params {
        let converted = serde_json::to_value(value).ok();
        if let Some(converted) = converted {
            params.insert(name.clone(), converted);
        }
    }
    match get_watsonx_credentials(
        &mut params,
        request.resolved.api_key.as_ref(),
        request.resolved.api_base.as_deref(),
        &watsonx_environment(request.environment),
    ) {
        Ok(credentials) => {
            if let Some(token) = credentials.token {
                mapped.insert(
                    "token".to_owned(),
                    Json::Text(token.expose_secret().to_owned()),
                );
            }
            Ok(Credentials {
                api_key: Some(credentials.api_key),
                api_base: Some(credentials.api_base),
                headers: request.headers,
            })
        }
        Err(error) => Err(watsonx_failure(&error, request.model, request.provider)),
    }
}
fn watsonx_failure(error: &WatsonxError, model: &str, provider: &str) -> RawException {
    match error {
        WatsonxError::Api {
            status_code,
            message,
            ..
        } => provider_failure(*status_code, message, model, provider, "WatsonxError"),
        WatsonxError::IamStatus { status, .. } => {
            provider_failure(*status, &error.to_string(), model, provider, "WatsonxError")
        }
        WatsonxError::IamApiKeyRequired
        | WatsonxError::InvalidField { .. }
        | WatsonxError::InvalidHeader { .. }
        | WatsonxError::IamUrl(_)
        | WatsonxError::IamTransport(_)
        | WatsonxError::IamReply(_)
        | WatsonxError::IamReplyField { .. }
        | WatsonxError::Cache(_) => value_failure(&error.to_string(), model, provider),
    }
}
fn watsonx_environment(lookup: &dyn EnvironmentLookup) -> WatsonxEnvironment {
    let text = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
    let secret = |name: &str| text(name).map(SecretString::from);
    WatsonxEnvironment {
        watsonx_iam_url: text("WATSONX_IAM_URL"),
        wx_api_key: secret("WX_API_KEY"),
        watsonx_api_key: secret("WATSONX_API_KEY"),
        watsonx_apikey: secret("WATSONX_APIKEY"),
        watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
        watsonx_token: secret("WATSONX_TOKEN"),
        watsonx_api_base: text("WATSONX_API_BASE"),
        watsonx_url: text("WATSONX_URL"),
        wx_url: text("WX_URL"),
        wml_url: text("WML_URL"),
        watsonx_project_id: text("WATSONX_PROJECT_ID"),
        wx_project_id: text("WX_PROJECT_ID"),
        project_id: text("PROJECT_ID"),
        watsonx_region: text("WATSONX_REGION"),
        wx_region: text("WX_REGION"),
        region: text("REGION"),
        watsonx_deployment_space_id: text("WATSONX_DEPLOYMENT_SPACE_ID"),
        watsonx_space_id: text("WATSONX_SPACE_ID"),
        wx_space_id: text("WX_SPACE_ID"),
        space_id: text("SPACE_ID"),
    }
}

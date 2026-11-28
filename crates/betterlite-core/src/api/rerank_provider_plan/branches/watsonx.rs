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
use secrecy::ExposeSecret as _;
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
) -> Result<Credentials, Box<RawException>> {
    let mut params = WatsonxParams::new();
    for (name, value) in request.generic_params {
        if let Ok(converted) = serde_json::to_value(value) {
            params.insert(name.clone(), converted);
        }
    }
    match get_watsonx_credentials(
        &mut params,
        request.resolved.api_key.as_ref(),
        request.resolved.api_base.as_deref(),
        &WatsonxEnvironment::from_lookup(request.environment),
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
        Err(error) => Err(Box::new(watsonx_failure(
            &error,
            request.model,
            request.provider,
        ))),
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

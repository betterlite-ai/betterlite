use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_urls::{EndpointSelection, select_base_url_or_endpoint};
use crate::credentials::source::CredentialSources;
use crate::endpoints::projection::{ClientKind, HttpClient};
use crate::oidc_exchange::OidcExchangeInput;

use super::error::{Raised, Reported};
use super::records::{PreCallRecord, StreamCall, StreamClient};

pub const SYNC_CLIENT_MESSAGE: &str = "azure_client is not an instance of AzureOpenAI or OpenAI";
pub const ASYNC_CLIENT_MESSAGE: &str =
    "Azure client is not an instance of AsyncAzureOpenAI or AsyncOpenAI";
pub const OIDC_PREFIX: &str = "oidc/";

// The credential entry select_azure_base_url_or_endpoint leaves in the sync map
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamCredential {
    ApiKey,
    AzureAdToken(SecretString),
    AzureAdTokenProvider(u64),
    Unset,
}

// azure_client_params as azure.py's streaming() builds it and then never passes on
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnusedClientParams {
    pub api_version: String,
    pub endpoint: EndpointSelection,
    pub azure_deployment: String,
    pub http_client: HttpClient,
    pub max_retries: i64,
    pub timeout: Option<f64>,
    pub credential: StreamCredential,
}

impl UnusedClientParams {
    // The map's azure_ad_token is the local variable azure.py logs; without one, the caller's token is
    #[must_use]
    pub fn logged_token(&self) -> Option<&SecretString> {
        match &self.credential {
            StreamCredential::AzureAdToken(token) => Some(token),
            StreamCredential::ApiKey
            | StreamCredential::AzureAdTokenProvider(_)
            | StreamCredential::Unset => None,
        }
    }
}

// The whole map is discarded: get_azure_openai_client only gets the arguments the caller passed.
pub fn unused_client_params(
    call: &StreamCall<'_>,
    sources: &dyn CredentialSources,
) -> Result<UnusedClientParams, Raised> {
    let endpoint = select_base_url_or_endpoint(EndpointSelection {
        azure_endpoint: Some(call.api_base.to_owned()),
        base_url: None,
    });
    let credential = credential_entry(call, sources)?;
    Ok(UnusedClientParams {
        api_version: call.api_version.to_owned(),
        endpoint,
        azure_deployment: call.model.to_owned(),
        http_client: HttpClient {
            is_async: false,
            shared: true,
        },
        max_retries: call.max_retries,
        timeout: call.timeout,
        credential,
    })
}

fn credential_entry(
    call: &StreamCall<'_>,
    sources: &dyn CredentialSources,
) -> Result<StreamCredential, Raised> {
    if call.api_key.is_some() {
        return Ok(StreamCredential::ApiKey);
    }
    if let Some(token) = call.azure_ad_token {
        if token.expose_secret().starts_with(OIDC_PREFIX) {
            let exchanged = sources
                .oidc(&OidcExchangeInput {
                    azure_ad_token: token.clone(),
                    azure_client_id: None,
                    azure_tenant_id: None,
                    scope: None,
                })
                .map_err(|failure| {
                    Raised::raised(
                        failure.class,
                        Reported::Present(failure.status),
                        Reported::Present(Some(failure.message.clone())),
                        failure.message,
                    )
                })?;
            return Ok(StreamCredential::AzureAdToken(exchanged));
        }
        return Ok(StreamCredential::AzureAdToken(token.clone()));
    }
    if let Some(provider) = call.azure_ad_token_provider {
        return Ok(StreamCredential::AzureAdTokenProvider(provider.identity));
    }
    Ok(StreamCredential::Unset)
}

pub fn require_sync(client: &StreamClient) -> Result<(), Raised> {
    match client.kind {
        ClientKind::AzureOpenAi | ClientKind::OpenAi => Ok(()),
        ClientKind::AsyncAzureOpenAi | ClientKind::AsyncOpenAi | ClientKind::Other => {
            Err(Raised::azure(500, SYNC_CLIENT_MESSAGE.to_owned()))
        }
    }
}

pub fn require_async(client: &StreamClient) -> Result<(), Raised> {
    match client.kind {
        ClientKind::AsyncAzureOpenAi | ClientKind::AsyncOpenAi => Ok(()),
        ClientKind::AzureOpenAi | ClientKind::OpenAi | ClientKind::Other => {
            Err(Raised::value_error(ASYNC_CLIENT_MESSAGE.to_owned()))
        }
    }
}

fn secret_json(value: Option<&SecretString>) -> Json {
    value.map_or(Json::Null, |secret| {
        Json::Text(secret.expose_secret().to_owned())
    })
}

// logging_obj.pre_call(input=data["messages"], api_key=azure_client.api_key, additional_args=...)
pub fn pre_call(
    call: &StreamCall<'_>,
    client: &StreamClient,
    token: Option<&SecretString>,
) -> Result<PreCallRecord, Raised> {
    let input = call
        .data
        .get("messages")
        .cloned()
        .ok_or_else(|| Raised::key_error("messages"))?;
    let headers = IndexMap::from([
        ("api_key".to_owned(), secret_json(call.api_key)),
        ("azure_ad_token".to_owned(), secret_json(token)),
    ]);
    let passthrough = IndexMap::from([("acompletion".to_owned(), Json::Bool(true))]);
    Ok(PreCallRecord {
        input,
        api_key: client.api_key.clone(),
        additional_args: AdditionalArgs {
            api_base: Some(call.api_base.to_owned()),
            headers: Some(headers),
            complete_input_dict: Some(RawBody::Object(call.data.clone())),
            request_str: None,
            openai_sdk: None,
            passthrough,
        },
    })
}

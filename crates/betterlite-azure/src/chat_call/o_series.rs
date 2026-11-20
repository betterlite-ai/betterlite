use std::future::Future;
use std::pin::Pin;

use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;

use crate::endpoints::initialize::EndpointError;
use crate::endpoints::projection::LiteLlmParams;

use super::client::{ClientHandle, ClientSource, GetClient};

// AzureOpenAIO1ChatCompletion.completion's arguments, forwarded unchanged to OpenAIChatCompletion.completion
pub struct OSeriesCall<'call, Client> {
    pub model: Option<&'call str>,
    pub messages: Option<&'call [Message]>,
    pub api_key: Option<&'call SecretString>,
    pub api_base: Option<&'call str>,
    pub api_version: Option<&'call str>,
    pub azure_ad_token: Option<&'call SecretString>,
    pub timeout: Option<f64>,
    pub optional_params: &'call Object,
    pub litellm_params: LiteLlmParams,
    pub acompletion: bool,
    pub headers: Option<&'call SecretHeaders>,
    pub organization: Option<&'call str>,
    pub drop_params: Option<bool>,
    pub shared_session: Option<&'call str>,
    pub dynamic_params: Option<bool>,
    pub supplied: Option<ClientHandle<Client>>,
}

// Everything super().completion receives, with the Azure client this handler built first
pub struct OSeriesForward<'call, Client> {
    pub client: ClientHandle<Client>,
    pub model: Option<&'call str>,
    pub messages: Option<&'call [Message]>,
    pub api_key: Option<&'call SecretString>,
    pub api_base: Option<&'call str>,
    pub api_version: Option<&'call str>,
    pub azure_ad_token: Option<&'call SecretString>,
    pub timeout: Option<f64>,
    pub optional_params: &'call Object,
    pub litellm_params: &'call LiteLlmParams,
    pub acompletion: bool,
    pub headers: Option<&'call SecretHeaders>,
    pub organization: Option<&'call str>,
    pub drop_params: Option<bool>,
    pub shared_session: Option<&'call str>,
    pub dynamic_params: Option<bool>,
}

// B004's chat completion handler, reached through the seam this handler delegates to
pub trait OSeriesDelegate: Send + Sync {
    type Client: Send + Sync + Clone;
    type Reply: Send;
    type Error: Send;

    fn completion<'call>(
        &'call self,
        call: OSeriesForward<'call, Self::Client>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Reply, Self::Error>> + Send + 'call>>;
}

#[derive(Debug)]
pub enum OSeriesError<Error> {
    Client(EndpointError),
    Delegate(Error),
}

// AzureOpenAIO1ChatCompletion.completion: no model reaches get_azure_openai_client and no fake streaming lives here
pub async fn completion<'forward, 'call, Source, Delegate>(
    source: &Source,
    call: &'forward OSeriesCall<'call, Source::Client>,
    delegate: &Delegate,
) -> Result<Delegate::Reply, OSeriesError<Delegate::Error>>
where
    Source: ClientSource,
    Delegate: OSeriesDelegate<Client = Source::Client>,
    'call: 'forward,
{
    let client = source
        .get(GetClient {
            api_key: call.api_key,
            api_base: call.api_base,
            api_version: call.api_version,
            model: None,
            litellm_params: &call.litellm_params,
            is_async: call.acompletion,
            supplied: call.supplied.as_ref(),
        })
        .map_err(OSeriesError::Client)?;
    delegate
        .completion(OSeriesForward {
            client,
            model: call.model,
            messages: call.messages,
            api_key: call.api_key,
            api_base: call.api_base,
            api_version: call.api_version,
            azure_ad_token: call.azure_ad_token,
            timeout: call.timeout,
            optional_params: call.optional_params,
            litellm_params: &call.litellm_params,
            acompletion: call.acompletion,
            headers: call.headers,
            organization: call.organization,
            drop_params: call.drop_params,
            shared_session: call.shared_session,
            dynamic_params: call.dynamic_params,
        })
        .await
        .map_err(OSeriesError::Delegate)
}

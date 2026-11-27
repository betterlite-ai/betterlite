use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::user_url_policy::Settings as UserUrlPolicy;
use betterlite_config::tls::Verification;
use betterlite_http::gigachat_tokens::error::GigachatAuthError;
use betterlite_http::gigachat_tokens::request::GigachatTokenRequest;
use betterlite_http::gigachat_tokens::tokens::GigachatTokens;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use betterlite_openai::gigachat_chat::config::{
    GigachatBearer, GigachatChatConfig, GigachatImageUploader,
};
use betterlite_openai::gigachat_chat::error::GigaChatError;
use betterlite_openai::gigachat_files::download::ImageDownloader;
use betterlite_openai::gigachat_files::{
    GigachatFileTransport, GigachatFileUploader, GigachatFilesConfig,
};
use betterlite_openai::gigachat_utils::{IdSource, UuidIds};
use secrecy::SecretString;

static UUID_IDS: UuidIds = UuidIds;

// B8177's bearer over B7325's token service: the service swaps the credentials for a token and caches it
pub struct GigachatTokenBearer {
    tokens: Arc<dyn GigachatTokens>,
}

impl GigachatTokenBearer {
    pub const fn new(tokens: Arc<dyn GigachatTokens>) -> Self {
        Self { tokens }
    }
}

#[async_trait]
impl GigachatBearer for GigachatTokenBearer {
    async fn access_token(
        &self,
        credentials: Option<&SecretString>,
    ) -> Result<SecretString, GigaChatError> {
        let request = GigachatTokenRequest {
            credentials: credentials.cloned(),
            ..GigachatTokenRequest::default()
        };
        self.tokens.access_token(&request).await.map_err(auth_error)
    }
}

fn auth_error(error: GigachatAuthError) -> GigaChatError {
    GigaChatError {
        status_code: error.status_code().as_u16(),
        message: error.to_string(),
        headers: None,
        source: None,
    }
}

// the upload goes out on the outbound client, which verifies the operator's certificate
pub struct ProxiedGigachatTransport(pub ProxiedClient);

impl GigachatFileTransport for ProxiedGigachatTransport {
    async fn upload(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        self.0.send_buffered(request).await
    }
}

pub struct GigachatFilesSettings<'settings> {
    pub api_base: Option<&'settings str>,
    pub api_base_env: Option<&'settings str>,
    pub credentials: Option<&'settings SecretString>,
}

// B9996 takes the download out of the transport; the uploader holds the downloader built from the user url policy
pub fn uploader(
    client: &ProxiedClient,
    policy: &UserUrlPolicy,
    bearer: Arc<dyn GigachatBearer>,
    ids: Arc<dyn IdSource>,
    settings: &GigachatFilesSettings<'_>,
    capacity: usize,
) -> GigachatFileUploader<ProxiedGigachatTransport, SystemResolver> {
    let config = GigachatFilesConfig {
        bearer,
        ids,
        credentials: settings.credentials.cloned(),
        api_base: settings.api_base.map(str::to_owned),
        api_base_env: settings.api_base_env.map(str::to_owned),
        downloader: image_downloader(client, policy),
    };
    GigachatFileUploader::new(ProxiedGigachatTransport(client.clone()), config, capacity)
}

// the downloader fetches an image url under the user url policy and the outbound client's body limit
pub fn image_downloader(
    client: &ProxiedClient,
    policy: &UserUrlPolicy,
) -> ImageDownloader<SystemResolver> {
    ImageDownloader {
        urls: UserUrlFetcher {
            validator: UrlValidator::new(
                SystemResolver,
                &policy.user_url_allowed_hosts,
                &Verification::Enabled,
            ),
            validation: policy.user_url_validation,
        },
        client: client.clone(),
    }
}

// the chat config B8177's chat path takes, with the uploader the caller keeps alive across requests
pub fn chat_config<'deps>(
    bearer: &'deps dyn GigachatBearer,
    uploader: &'deps dyn GigachatImageUploader,
) -> GigachatChatConfig<'deps> {
    GigachatChatConfig {
        bearer,
        uploader,
        ids: &UUID_IDS,
    }
}

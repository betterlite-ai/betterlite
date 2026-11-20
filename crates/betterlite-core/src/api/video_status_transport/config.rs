use async_trait::async_trait;

use super::clients::ClientFactories;
use super::error::{BaseLlmException, ProviderFailure};
use super::records::{Headers, Params, VideoEnvironment, VideoReply, VideoRequestContext};
use crate::api::video_records::VideoObject;

// The provider's own error class, the one fact B2402's converter asks the status config for
pub trait VideoErrorClasses: Send + Sync {
    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> BaseLlmException;
}

// LiteLLM's BaseVideoConfig narrowed to the status call: the methods its status handler reaches, with the provider's own client types
#[async_trait]
pub trait VideoStatusConfig: VideoErrorClasses {
    // B3331's HTTPHandler and AsyncHTTPHandler, which a provider's status transforms accept
    type SyncClient;
    type AsyncClient: Sync;

    // LiteLLM lets this raise, before its try block
    fn validate_environment(
        &self,
        request: VideoEnvironment<'_>,
    ) -> Result<Headers, ProviderFailure>;

    fn get_complete_url(
        &self,
        model: &str,
        api_base: Option<&str>,
        litellm_params: &Params,
    ) -> Result<String, ProviderFailure>;

    // ast-grep-ignore: no-long-names
    fn transform_video_status_retrieve_request(
        &self,
        video_id: &str,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), ProviderFailure>;

    // ast-grep-ignore: no-long-names
    fn transform_video_status_retrieve_response(
        &self,
        reply: VideoReply<'_>,
        client: Option<&Self::SyncClient>,
    ) -> Result<VideoObject, ProviderFailure>;

    // ast-grep-ignore: no-long-names
    async fn async_transform_video_status_retrieve_response(
        &self,
        reply: VideoReply<'_>,
        client: Option<&Self::AsyncClient>,
    ) -> Result<VideoObject, ProviderFailure>;
}

// The status config object the handler drives, carrying the client types its transforms accept
pub type StatusConfig<'call, Factories> = dyn VideoStatusConfig<
        SyncClient = <Factories as ClientFactories>::Synchronous,
        AsyncClient = <Factories as ClientFactories>::Asynchronous,
    > + 'call;

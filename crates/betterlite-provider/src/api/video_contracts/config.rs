use std::error::Error;

use async_trait::async_trait;
use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_records::{
    CharacterObject, VideoCreateOptionalRequestParams, VideoObject,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};

use super::error::VideoContractError;
use super::records::{
    Headers, Params, RequestFiles, VideoEditInput, VideoEnvironment, VideoExtensionInput,
    VideoListQuery, VideoLogging, VideoReply, VideoRequestContext,
};

// LiteLLM's BaseVideoConfig: providers fill the abstract transforms and keep the inherited defaults for the rest
#[async_trait]
pub trait BaseVideoConfig: Send + Sync {
    type Error: Error + Send + Sync + 'static + From<VideoContractError>;
    // LiteLLM's HTTPHandler and AsyncHTTPHandler, which the transport tasks own
    type SyncClient;
    type AsyncClient: Sync;

    fn get_supported_openai_params(&self, model: &str) -> Vec<String>;

    fn map_openai_params(
        &self,
        video_create_optional_params: &VideoCreateOptionalRequestParams,
        model: &str,
        drop_params: bool,
    ) -> Params;

    // The base body is the empty mapping, which is also what Default builds
    #[cfg_attr(test, mutants::skip)]
    fn validate_environment(&self, _request: VideoEnvironment<'_>) -> Headers {
        Headers::new()
    }

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, Self::Error> {
        let Some(base) = api_base else {
            return Err(Self::Error::from(VideoContractError::ApiBaseRequired));
        };
        Ok(base.to_owned())
    }

    fn use_multipart_form_data(&self) -> bool {
        false
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_create_request(
        &self,
        model: &str,
        prompt: &str,
        optional_params: &VideoCreateOptionalRequestParams,
        context: VideoRequestContext<'_>,
    ) -> Result<(Params, RequestFiles, String), Self::Error>;

    // ast-grep-ignore: no-long-names
    fn transform_video_create_response(
        &self,
        model: &str,
        reply: VideoReply<'_>,
        request_data: Option<&Params>,
    ) -> Result<VideoObject, Self::Error>;

    // ast-grep-ignore: no-long-names
    fn transform_video_content_request(
        &self,
        video_id: &str,
        variant: Option<&str>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), Self::Error>;

    // ast-grep-ignore: no-long-names
    fn transform_video_content_response(
        &self,
        raw_response: &UpstreamResponse,
        logging_obj: &mut VideoLogging,
    ) -> Result<Vec<u8>, Self::Error>;

    // ast-grep-ignore: no-long-names
    async fn async_transform_video_content_response(
        &self,
        raw_response: &UpstreamResponse,
        logging_obj: &mut VideoLogging,
    ) -> Result<Vec<u8>, Self::Error> {
        self.transform_video_content_response(raw_response, logging_obj)
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_remix_request(
        &self,
        video_id: &str,
        prompt: &str,
        extra_body: Option<&Params>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), Self::Error>;

    // ast-grep-ignore: no-long-names
    fn transform_video_remix_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<VideoObject, Self::Error>;

    // ast-grep-ignore: no-long-names
    fn transform_video_list_request(
        &self,
        query: VideoListQuery<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), Self::Error>;

    // ast-grep-ignore: no-long-names
    fn transform_video_list_response(&self, reply: VideoReply<'_>) -> Result<Params, Self::Error>;

    // ast-grep-ignore: no-long-names
    fn transform_video_status_retrieve_request(
        &self,
        video_id: &str,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), Self::Error>;

    // ast-grep-ignore: no-long-names
    fn transform_video_status_retrieve_response(
        &self,
        reply: VideoReply<'_>,
        client: Option<&Self::SyncClient>,
    ) -> Result<VideoObject, Self::Error>;

    // ast-grep-ignore: no-long-names
    async fn async_transform_video_status_retrieve_response(
        &self,
        reply: VideoReply<'_>,
        _client: Option<&Self::AsyncClient>,
    ) -> Result<VideoObject, Self::Error> {
        // The sync transform gets no client: only an async override uses it, to fetch the artifact
        self.transform_video_status_retrieve_response(reply, None)
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_create_character_request(
        &self,
        _name: &str,
        _video: &SuppliedMedia,
        _context: VideoRequestContext<'_>,
    ) -> Result<(String, RequestFiles), Self::Error> {
        Err(Self::Error::from(
            VideoContractError::CreateCharacterUnsupported,
        ))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_create_character_response(
        &self,
        _raw_response: &UpstreamResponse,
        _logging_obj: &mut VideoLogging,
    ) -> Result<CharacterObject, Self::Error> {
        Err(Self::Error::from(
            VideoContractError::CreateCharacterUnsupported,
        ))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_get_character_request(
        &self,
        _character_id: &str,
        _context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), Self::Error> {
        Err(Self::Error::from(
            VideoContractError::GetCharacterUnsupported,
        ))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_get_character_response(
        &self,
        _raw_response: &UpstreamResponse,
        _logging_obj: &mut VideoLogging,
    ) -> Result<CharacterObject, Self::Error> {
        Err(Self::Error::from(
            VideoContractError::GetCharacterUnsupported,
        ))
    }

    // None means the edit needs no pre-fetch call, and Some is the (url, body) to send first
    fn get_video_edit_prefetch_params(
        &self,
        _video_id: &str,
        _context: VideoRequestContext<'_>,
    ) -> Option<(String, Params)> {
        None
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_edit_request(
        &self,
        _input: VideoEditInput<'_>,
        _context: VideoRequestContext<'_>,
    ) -> Result<(String, Params, Option<RequestFiles>), Self::Error> {
        Err(Self::Error::from(VideoContractError::EditUnsupported))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_edit_response(
        &self,
        _reply: VideoReply<'_>,
        _request_data: Option<&Params>,
    ) -> Result<VideoObject, Self::Error> {
        Err(Self::Error::from(VideoContractError::EditUnsupported))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_extension_request(
        &self,
        _input: VideoExtensionInput<'_>,
        _context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), Self::Error> {
        Err(Self::Error::from(VideoContractError::ExtensionUnsupported))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_extension_response(
        &self,
        _reply: VideoReply<'_>,
    ) -> Result<VideoObject, Self::Error> {
        Err(Self::Error::from(VideoContractError::ExtensionUnsupported))
    }

    // B2202's BaseLLMException with the three supplied facts; LiteLLM raises it where this returns it
    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> ProviderError {
        ProviderError::from(ProviderErrorParts {
            status_code,
            message: error_message.to_owned(),
            headers: Some(
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            ),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}

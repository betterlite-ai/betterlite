use betterlite_openai::error::ProviderError;

use crate::api::video_records::VideoObject;

use super::error::{ConfigFailure, ErrorHeaders};
use super::records::{
    Headers, Params, VideoEnvironment, VideoListQuery, VideoReply, VideoRequestContext,
};

// The B3331 BaseVideoConfig members these handlers call: B3331 lives in betterlite-provider, which depends on this crate, so a provider config reaches the handlers through an adapter implementing this trait, as B1066 adapts WriteConfig
pub trait VideoRemixListConfig: Send + Sync {
    fn validate_environment(&self, request: VideoEnvironment<'_>) -> Headers;

    fn get_complete_url(
        &self,
        model: &str,
        api_base: Option<&str>,
        litellm_params: &Params,
    ) -> Result<String, ConfigFailure>;

    fn transform_video_remix_request(
        &self,
        video_id: &str,
        prompt: &str,
        extra_body: Option<&Params>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), ConfigFailure>;

    fn transform_video_remix_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<VideoObject, ConfigFailure>;

    fn transform_video_list_request(
        &self,
        query: VideoListQuery<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), ConfigFailure>;

    fn transform_video_list_response(&self, reply: VideoReply<'_>)
    -> Result<Params, ConfigFailure>;

    // B3331's get_error_class: LiteLLM raises what this returns, and the handler returns it as the typed error
    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: ErrorHeaders,
    ) -> ProviderError;
}

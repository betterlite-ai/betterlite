use betterlite_core::api::video_edit_call::VideoEditConfigLookup;
use betterlite_core::api::video_edit_transport::config::{
    BaseVideoConfig, Headers, Params, RequestFiles, VideoEditInput, VideoEnvironment,
    VideoExtensionInput, VideoReply, VideoRequestContext,
};
use betterlite_core::api::video_edit_transport::error::BoxError;
use betterlite_core::api::video_extension_call::VideoExtensionConfigLookup;
use betterlite_core::api::video_records::VideoObject;
use betterlite_core::api::video_selection::VideoConfigTag;
use betterlite_openai::error::ProviderError;
use betterlite_openai::video_edits::edit::{
    VideoEditRequest, transform_video_edit_request as edit_request,
};
use betterlite_openai::video_edits::extension::VideoExtensionRequest;
use betterlite_openai::video_edits::extension::transform_video_extension_request as extension_request; // ast-grep-ignore: no-long-names
use betterlite_openai::video_edits::response::{
    VideoReply as OpenAiReply, transform_video_response as video_response,
};
use betterlite_openai::videos::config::complete_url;
use betterlite_openai::videos::error::get_error_class as provider_error_class;

use super::{OpenAiEditConfig, media_body, named_part, record};

impl VideoEditConfigLookup for OpenAiEditConfig {
    fn video_config(&self, tag: &VideoConfigTag) -> Option<&dyn BaseVideoConfig> {
        matches!(tag, VideoConfigTag::OpenAi).then_some(self)
    }
}

impl VideoExtensionConfigLookup for OpenAiEditConfig {
    fn video_config(&self, tag: &VideoConfigTag) -> Option<&dyn BaseVideoConfig> {
        matches!(tag, VideoConfigTag::OpenAi).then_some(self)
    }
}

fn edited_reply(reply: &VideoReply<'_>) -> Result<VideoObject, BoxError> {
    let reply_object = video_response(&OpenAiReply {
        body: &reply.raw_response.body,
        custom_llm_provider: reply.custom_llm_provider,
        request_data: None,
    })?;
    let object: VideoObject = record(&reply_object)?;
    Ok(object)
}

fn edit_wire<'call>(
    input: VideoEditInput<'call>,
    context: VideoRequestContext<'_>,
) -> VideoEditRequest<'call> {
    VideoEditRequest {
        prompt: input.prompt.to_owned(),
        video_id: input.video_id.to_owned(),
        api_base: context.api_base.to_owned(),
        video_file: input.video_file.map(media_body),
        extra_body: input.extra_body.cloned(),
        prefetched_source_data: None,
    }
}

// the URL and body an extension sends: its wire fields, then the source transform
fn extension_call(
    input: VideoExtensionInput<'_>,
    context: VideoRequestContext<'_>,
) -> (String, Params) {
    let request = VideoExtensionRequest {
        prompt: input.prompt.to_owned(),
        video_id: input.video_id.to_owned(),
        seconds: input.seconds.to_owned(),
        api_base: context.api_base.to_owned(),
        extra_body: input.extra_body.cloned(),
    };
    let extension = extension_request(request);
    (extension.url, extension.body)
}

impl BaseVideoConfig for OpenAiEditConfig {
    fn validate_environment(&self, request: VideoEnvironment<'_>) -> Headers {
        self.edit_headers(request)
    }

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, BoxError> {
        Ok(complete_url(api_base))
    }

    fn get_video_edit_prefetch_params(
        &self,
        _video_id: &str,
        _context: VideoRequestContext<'_>,
    ) -> Option<(String, Params)> {
        None
    }

    fn transform_video_edit_request(
        &self,
        input: VideoEditInput<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params, Option<RequestFiles>), BoxError> {
        let edit = edit_request(edit_wire(input, context));
        let files = match edit.files {
            Some(part) => Some(vec![named_part(part, self.video_name.as_deref())?]),
            None => None,
        };
        Ok((edit.url, edit.body, files))
    }

    fn transform_video_edit_response(
        &self,
        reply: VideoReply<'_>,
        _request_data: Option<&Params>,
    ) -> Result<VideoObject, BoxError> {
        edited_reply(&reply)
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_extension_request(
        &self,
        input: VideoExtensionInput<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), BoxError> {
        Ok(extension_call(input, context))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_extension_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<VideoObject, BoxError> {
        edited_reply(&reply)
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> ProviderError {
        let pairs = headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        provider_error_class(error_message, status_code, pairs)
    }
}

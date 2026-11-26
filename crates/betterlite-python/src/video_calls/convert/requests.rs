use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_content_call::VideoContentRequest;
use betterlite_core::api::video_generation_call::VideoGenerationRequest;
use betterlite_core::api::video_list_call::VideoListCall;
use betterlite_core::api::video_remix_call::VideoRemixCall;
use betterlite_core::api::video_status_call::VideoStatusRequest;
use betterlite_models::decisions::Json;
use betterlite_sdk::call_setup::RequestMap;
use secrecy::SecretString;

use super::fields::{
    INTEGER, STRING, duration, failed, meta, number, params, seconds, text, texts,
};
use super::{
    CONTENT_NAMED, ConvertError, GENERATION_NAMED, LIST_NAMED, REMIX_NAMED, Reference,
    STATUS_NAMED, loose,
};

// Flat core request records require common fields to be expanded into each constructor.
macro_rules! read_request {
    ($arguments:ident, $named:ident, $($field:ident: $value:expr,)*) => {
        Ok(Self {
            video_id: text($arguments, "video_id")?.ok_or_else(|| failed("video_id", STRING))?,
            custom_llm_provider: text($arguments, "custom_llm_provider")?,
            extra_headers: texts($arguments, "extra_headers")?,
            extra_query: params($arguments, "extra_query")?,
            extra_body: params($arguments, "extra_body")?,
            kwargs: loose($arguments, &$named),
            $($field: $value,)*
        })
    };
}

macro_rules! provider_call {
    ($arguments:ident, $named:ident, $($field:ident: $value:expr,)*) => {
        Ok(Self {
            custom_llm_provider: text($arguments, "custom_llm_provider")?,
            api_key: text($arguments, "api_key")?.map(SecretString::from),
            api_base: text($arguments, "api_base")?,
            api_version: text($arguments, "api_version")?,
            extra_headers: texts($arguments, "extra_headers")?,
            extra_query: params($arguments, "extra_query")?,
            extra_body: params($arguments, "extra_body")?,
            mock_response: $arguments.get("mock_response").cloned(),
            litellm_metadata: meta($arguments, "litellm_metadata")?,
            litellm_call_id: text($arguments, "litellm_call_id")?,
            user: text($arguments, "user")?,
            timeout: number($arguments, "timeout")?,
            extra: loose($arguments, &$named),
            $($field: $value,)*
        })
    };
}

impl Build for VideoGenerationRequest {
    fn build(arguments: &RequestMap, media: Option<Reference>) -> Result<Self, ConvertError> {
        let mut generation =
            Self::new(text(arguments, "prompt")?.ok_or_else(|| failed("prompt", STRING))?);
        generation.model = text(arguments, "model")?;
        generation.seconds = seconds(arguments, "seconds")?;
        generation.size = text(arguments, "size")?;
        generation.user = text(arguments, "user")?;
        generation.timeout = number(arguments, "timeout")?;
        generation.custom_llm_provider = text(arguments, "custom_llm_provider")?;
        generation.extra_headers = texts(arguments, "extra_headers")?;
        generation.extra_query = params(arguments, "extra_query")?;
        generation.extra_body = params(arguments, "extra_body")?;
        generation.input_reference = media.map(|reference| SuppliedMedia::Bytes(reference.content));
        generation.kwargs = loose(arguments, &GENERATION_NAMED);
        Ok(generation)
    }
}

impl Build for VideoContentRequest {
    fn build(arguments: &RequestMap, _media: Option<Reference>) -> Result<Self, ConvertError> {
        read_request!(arguments, CONTENT_NAMED,
            timeout: duration(number(arguments, "timeout")?)?,
            variant: text(arguments, "variant")?,
        )
    }
}

impl Build for VideoRemixCall {
    fn build(arguments: &RequestMap, _media: Option<Reference>) -> Result<Self, ConvertError> {
        provider_call!(arguments, REMIX_NAMED,
            video_id: text(arguments, "video_id")?.ok_or_else(|| failed("video_id", STRING))?,
            prompt: text(arguments, "prompt")?.ok_or_else(|| failed("prompt", STRING))?,
        )
    }
}

impl Build for VideoListCall {
    fn build(arguments: &RequestMap, _media: Option<Reference>) -> Result<Self, ConvertError> {
        let pagination_limit = match arguments.get("limit") {
            None | Some(Json::Null) => None,
            Some(Json::Number(value)) => {
                Some(value.as_i64().ok_or_else(|| failed("limit", INTEGER))?)
            }
            Some(_) => return Err(failed("limit", INTEGER)),
        };
        provider_call!(arguments, LIST_NAMED,
            after: text(arguments, "after")?,
            limit: pagination_limit,
            order: text(arguments, "order")?,
        )
    }
}

impl Build for VideoStatusRequest {
    fn build(arguments: &RequestMap, _media: Option<Reference>) -> Result<Self, ConvertError> {
        read_request!(arguments, STATUS_NAMED,
            timeout: Some(duration(number(arguments, "timeout")?)?.unwrap_or_default()),
        )
    }
}

pub(crate) trait Build: Sized {
    fn build(arguments: &RequestMap, media: Option<Reference>) -> Result<Self, ConvertError>;
}

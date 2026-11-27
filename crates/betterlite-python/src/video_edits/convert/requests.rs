use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_character_calls::{
    CreateCharacterCall, DEFAULT_PROVIDER, GetCharacterCall,
};
use betterlite_core::api::video_edit_call::VideoEditCall;
use betterlite_core::api::video_edit_transport::config::Headers;
use betterlite_core::api::video_extension_call::VideoExtensionCall;
use betterlite_models::decisions::Json;
use betterlite_sdk::call_setup::RequestMap;
use indexmap::IndexMap;

use super::fields::{STRING, duration, failed, number, params, text, texts};
use super::{
    CREATE_NAMED, ConvertError, EDIT_NAMED, EXTENSION_NAMED, GET_NAMED, VideoFile, loose,
    video_media,
};

// both character operations declare the openai provider and no timeout of their own
fn provider(arguments: &RequestMap) -> Result<String, ConvertError> {
    match text(arguments, "custom_llm_provider")? {
        Some(named) => Ok(named),
        None => Ok(DEFAULT_PROVIDER.to_owned()),
    }
}

fn header_map(arguments: &RequestMap) -> Result<Option<BTreeMap<String, String>>, ConvertError> {
    let Some(entries) = texts(arguments, "extra_headers")? else {
        return Ok(None);
    };
    Ok(Some(entries.into_iter().collect()))
}

impl Build for CreateCharacterCall {
    fn build(arguments: &RequestMap, file: Option<&VideoFile>) -> Result<Self, ConvertError> {
        let video = video_media(arguments, file)?
            .map_or_else(|| SuppliedMedia::Text(String::new()), |found| found.media);
        Ok(Self {
            name: text(arguments, "name")?.ok_or_else(|| failed("name", STRING))?,
            video,
            timeout: number(arguments, "timeout")?,
            custom_llm_provider: provider(arguments)?,
            extra_headers: header_map(arguments)?,
            extra_query: None,
            extra_body: None,
            mock_response: arguments.get("mock_response").cloned(),
            litellm_call_id: text(arguments, "litellm_call_id")?,
            extra: loose(arguments, &CREATE_NAMED),
            is_async: false,
        })
    }
}

impl Build for GetCharacterCall {
    fn build(arguments: &RequestMap, _file: Option<&VideoFile>) -> Result<Self, ConvertError> {
        Ok(Self {
            character_id: text(arguments, "character_id")?
                .ok_or_else(|| failed("character_id", STRING))?,
            timeout: number(arguments, "timeout")?,
            custom_llm_provider: provider(arguments)?,
            extra_headers: header_map(arguments)?,
            extra_query: None,
            extra_body: None,
            mock_response: arguments.get("mock_response").cloned(),
            litellm_call_id: text(arguments, "litellm_call_id")?,
            extra: loose(arguments, &GET_NAMED),
            is_async: false,
        })
    }
}

// everything video_edit's handler call reads, owned so the pending half of an async call can outlive the payload
#[derive(Debug)]
pub(crate) struct OwnedEdit {
    video_id: String,
    prompt: String,
    timeout: Option<Duration>,
    custom_llm_provider: Option<String>,
    extra_headers: Option<Headers>,
    extra_body: Option<IndexMap<String, Json>>,
    video: Option<SuppliedMedia>,
    kwargs: IndexMap<String, Json>,
}

impl OwnedEdit {
    pub(crate) fn call(&self) -> VideoEditCall<'_> {
        VideoEditCall {
            video_id: &self.video_id,
            prompt: &self.prompt,
            timeout: self.timeout,
            custom_llm_provider: self.custom_llm_provider.as_deref(),
            extra_headers: self.extra_headers.as_ref(),
            extra_query: None,
            extra_body: self.extra_body.as_ref(),
            video: self.video.as_ref(),
            kwargs: &self.kwargs,
        }
    }
}

impl Build for OwnedEdit {
    fn build(arguments: &RequestMap, file: Option<&VideoFile>) -> Result<Self, ConvertError> {
        // @client hands the logging object in; the call cannot run its logging update without one
        let mut kwargs = loose(arguments, &EDIT_NAMED);
        kwargs.insert("litellm_logging_obj".to_owned(), Json::Bool(true));
        Ok(Self {
            video_id: text(arguments, "video_id")?.ok_or_else(|| failed("video_id", STRING))?,
            prompt: text(arguments, "prompt")?.ok_or_else(|| failed("prompt", STRING))?,
            timeout: duration(number(arguments, "timeout")?)?,
            custom_llm_provider: text(arguments, "custom_llm_provider")?,
            extra_headers: texts(arguments, "extra_headers")?,
            extra_body: params(arguments, "extra_body")?,
            video: video_media(arguments, file)?.map(|found| found.media),
            kwargs,
        })
    }
}

// everything video_extension's handler call reads, owned for the same reason
#[derive(Debug)]
pub(crate) struct OwnedExtension {
    video_id: String,
    prompt: String,
    seconds: String,
    timeout: Option<Duration>,
    custom_llm_provider: Option<String>,
    extra_headers: Option<Headers>,
    extra_body: Option<IndexMap<String, Json>>,
    kwargs: IndexMap<String, Json>,
}

impl OwnedExtension {
    pub(crate) fn call(&self) -> VideoExtensionCall<'_> {
        VideoExtensionCall {
            video_id: &self.video_id,
            prompt: &self.prompt,
            seconds: &self.seconds,
            timeout: self.timeout,
            custom_llm_provider: self.custom_llm_provider.as_deref(),
            extra_headers: self.extra_headers.as_ref(),
            extra_query: None,
            extra_body: self.extra_body.as_ref(),
            kwargs: &self.kwargs,
        }
    }
}

impl Build for OwnedExtension {
    fn build(arguments: &RequestMap, _file: Option<&VideoFile>) -> Result<Self, ConvertError> {
        let mut kwargs = loose(arguments, &EXTENSION_NAMED);
        kwargs.insert("litellm_logging_obj".to_owned(), Json::Bool(true));
        Ok(Self {
            video_id: text(arguments, "video_id")?.ok_or_else(|| failed("video_id", STRING))?,
            prompt: text(arguments, "prompt")?.ok_or_else(|| failed("prompt", STRING))?,
            seconds: text(arguments, "seconds")?.ok_or_else(|| failed("seconds", STRING))?,
            timeout: duration(number(arguments, "timeout")?)?,
            custom_llm_provider: text(arguments, "custom_llm_provider")?,
            extra_headers: texts(arguments, "extra_headers")?,
            extra_body: params(arguments, "extra_body")?,
            kwargs,
        })
    }
}

pub(crate) trait Build: Sized {
    fn build(arguments: &RequestMap, file: Option<&VideoFile>) -> Result<Self, ConvertError>;
}

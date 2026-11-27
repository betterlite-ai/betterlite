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

// each of the four argument objects is one keyword per field, read the same way in every handler
macro_rules! build_call {
    ($call:ident, $arguments:ident, $file:ident, { $($field:ident: $value:expr),* $(,)? }) => {
        impl Build for $call {
            fn build(
                $arguments: &RequestMap,
                $file: Option<&VideoFile>,
            ) -> Result<Self, ConvertError> {
                Ok(Self {
                    $($field: $value,)*
                })
            }
        }
    };
}

// the two owned requests hand the core the same view of their fields
macro_rules! call_view {
    ($owned:ident, $call:ident, $receiver:ident, { $($field:ident: $value:expr),* $(,)? }) => {
        impl $owned {
            pub(crate) fn call(&self) -> $call<'_> {
                let $receiver = self;
                $call {
                    $($field: $value,)*
                }
            }
        }
    };
}

fn header_map(arguments: &RequestMap) -> Result<Option<BTreeMap<String, String>>, ConvertError> {
    let Some(entries) = texts(arguments, "extra_headers")? else {
        return Ok(None);
    };
    Ok(Some(entries.into_iter().collect()))
}

// @client hands the logging object in; the call cannot run its logging update without one
fn edit_kwargs(arguments: &RequestMap, named: &[&str]) -> IndexMap<String, Json> {
    let mut kwargs = loose(arguments, named);
    kwargs.insert("litellm_logging_obj".to_owned(), Json::Bool(true));
    kwargs
}

build_call!(CreateCharacterCall, arguments, file, {
    name: text(arguments, "name")?.ok_or_else(|| failed("name", STRING))?,
    video: video_media(arguments, file)?
        .map_or_else(|| SuppliedMedia::Text(String::new()), |found| found.media),
    timeout: number(arguments, "timeout")?,
    custom_llm_provider: text(arguments, "custom_llm_provider")?
        .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned()),
    extra_headers: header_map(arguments)?,
    extra_query: None,
    extra_body: None,
    mock_response: arguments.get("mock_response").cloned(),
    litellm_call_id: text(arguments, "litellm_call_id")?,
    extra: loose(arguments, &CREATE_NAMED),
    is_async: false,
});

build_call!(GetCharacterCall, arguments, _file, {
    character_id: text(arguments, "character_id")?.ok_or_else(|| failed("character_id", STRING))?,
    timeout: number(arguments, "timeout")?,
    custom_llm_provider: text(arguments, "custom_llm_provider")?
        .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned()),
    extra_headers: header_map(arguments)?,
    extra_query: None,
    extra_body: None,
    mock_response: arguments.get("mock_response").cloned(),
    litellm_call_id: text(arguments, "litellm_call_id")?,
    extra: loose(arguments, &GET_NAMED),
    is_async: false,
});

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

call_view!(OwnedEdit, VideoEditCall, receiver, {
    video_id: receiver.video_id.as_str(),
    prompt: receiver.prompt.as_str(),
    timeout: receiver.timeout,
    custom_llm_provider: receiver.custom_llm_provider.as_deref(),
    extra_headers: receiver.extra_headers.as_ref(),
    extra_query: None,
    extra_body: receiver.extra_body.as_ref(),
    video: receiver.video.as_ref(),
    kwargs: &receiver.kwargs,
});

build_call!(OwnedEdit, arguments, file, {
    video_id: text(arguments, "video_id")?.ok_or_else(|| failed("video_id", STRING))?,
    prompt: text(arguments, "prompt")?.ok_or_else(|| failed("prompt", STRING))?,
    timeout: duration(number(arguments, "timeout")?)?,
    custom_llm_provider: text(arguments, "custom_llm_provider")?,
    extra_headers: texts(arguments, "extra_headers")?,
    extra_body: params(arguments, "extra_body")?,
    video: video_media(arguments, file)?.map(|found| found.media),
    kwargs: edit_kwargs(arguments, &EDIT_NAMED),
});

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

call_view!(OwnedExtension, VideoExtensionCall, receiver, {
    video_id: receiver.video_id.as_str(),
    prompt: receiver.prompt.as_str(),
    seconds: receiver.seconds.as_str(),
    timeout: receiver.timeout,
    custom_llm_provider: receiver.custom_llm_provider.as_deref(),
    extra_headers: receiver.extra_headers.as_ref(),
    extra_query: None,
    extra_body: receiver.extra_body.as_ref(),
    kwargs: &receiver.kwargs,
});

build_call!(OwnedExtension, arguments, _file, {
    video_id: text(arguments, "video_id")?.ok_or_else(|| failed("video_id", STRING))?,
    prompt: text(arguments, "prompt")?.ok_or_else(|| failed("prompt", STRING))?,
    seconds: text(arguments, "seconds")?.ok_or_else(|| failed("seconds", STRING))?,
    timeout: duration(number(arguments, "timeout")?)?,
    custom_llm_provider: text(arguments, "custom_llm_provider")?,
    extra_headers: texts(arguments, "extra_headers")?,
    extra_body: params(arguments, "extra_body")?,
    kwargs: edit_kwargs(arguments, &EXTENSION_NAMED),
});

pub(crate) trait Build: Sized {
    fn build(arguments: &RequestMap, file: Option<&VideoFile>) -> Result<Self, ConvertError>;
}

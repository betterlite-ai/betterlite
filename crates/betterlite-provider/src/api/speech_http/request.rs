use betterlite_openai::api::speech_provider_contract::TextToSpeechRequestData;
use betterlite_openai::openai_base_config::Object;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use url::Url;

use super::error::SpeechError;
use super::{
    ClientSource, Mode, Selected, SpeechCall, SpeechDeps, SpeechHeaders, TextToSpeechConfig,
};

// Which branch the send selects: dict_body first, then ssml_body, and neither is the ValueError inside the send try
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Payload {
    Dict(Object),
    Ssml(String),
    Missing,
}

// The selected httpx client class: the caller's client is used only when its kind matches the handler's
pub(super) fn select_client<'borrow, 'deps, S>(
    deps: &'borrow SpeechDeps<'deps, S>,
    mode: Mode,
    ssl_verify: Option<bool>,
) -> Result<Selected<'borrow, S::Client>, SpeechError>
where
    'deps: 'borrow,
    S: ClientSource,
{
    if let Some(supplied) = &deps.supplied
        && supplied.kind == mode
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    Ok(Selected::Created(deps.source.client(ssl_verify)?))
}

// validate_environment takes the caller's headers, and the handler overlays them again after it returns
pub(super) fn validated_headers(
    config: &dyn TextToSpeechConfig,
    call: &SpeechCall<'_>,
) -> SpeechHeaders {
    let mut caller_headers = SpeechHeaders::new();
    for (name, value) in call.extra_headers.into_iter().flatten() {
        caller_headers.insert(name.clone(), value.clone());
    }
    let mut headers = config.validate_environment(
        &caller_headers,
        call.model,
        call.params.api_key.as_ref(),
        call.params.api_base.as_deref(),
    );
    for (name, value) in &caller_headers {
        headers.insert(name.clone(), value.clone());
    }
    headers
}

pub(super) fn call_endpoint(
    config: &dyn TextToSpeechConfig,
    call: &SpeechCall<'_>,
) -> Result<(String, Url), SpeechError> {
    let api_base = config.complete_url(
        call.model,
        call.params.api_base.as_deref(),
        call.litellm_params,
    )?;
    let url = Url::parse(&api_base).map_err(|source| SpeechError::RequestUrl {
        url: api_base.clone(),
        source,
    })?;
    Ok((api_base, url))
}

// Provider-returned headers beat the caller's and the config's, credential names included
pub(super) fn request_headers(headers: &mut SpeechHeaders, data: &TextToSpeechRequestData) {
    for (name, value) in data.headers.iter().flatten() {
        headers.insert(name.clone(), value.clone());
    }
}

pub(super) fn payload(data: &TextToSpeechRequestData) -> Payload {
    if let Some(body) = &data.dict_body {
        return Payload::Dict(body.clone());
    }
    match &data.ssml_body {
        Some(text) => Payload::Ssml(text.clone()),
        None => Payload::Missing,
    }
}

pub(super) fn wire_headers(headers: &SpeechHeaders) -> Result<HeaderMap, SpeechError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| SpeechError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| SpeechError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

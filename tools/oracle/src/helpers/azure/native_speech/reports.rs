use std::collections::BTreeMap;

use betterlite_azure::native_speech::endpoint::get_complete_url;
use betterlite_azure::native_speech::headers::validate_environment;
use betterlite_azure::native_speech::identity::NativeSpeechIdentity;
use betterlite_azure::native_speech::mapping::{
    MappedParams, SpeechKwargs, SpeechOptionalParams, map_openai_params,
};
use betterlite_azure::native_speech::request::{
    SpeechRequestData, SpeechTransform, transform_request,
};
use betterlite_azure::native_speech::response::{HttpxBinaryResponseContent, RawSpeechResponse};
use betterlite_prompts::arguments::json::Json;
use secrecy::SecretString;
use serde_json::json;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

use super::{
    Bases, BinaryCases, Formats, HeaderCases, Input, Speeds, StyleCases, Texts, VoiceCases, Voices,
};

struct Prepared {
    mapped_voice: Option<String>,
    mapped: MappedParams,
    data: SpeechRequestData,
}

impl Prepared {
    fn new(
        text: &str,
        voice: Option<&Json>,
        optional: &SpeechOptionalParams,
        kwargs: &SpeechKwargs,
    ) -> Result<Self, Box<HelperError>> {
        let (mapped_voice, mapped) = map_openai_params(voice, optional, kwargs).or_value_error()?;
        let data = transform_request(SpeechTransform {
            input: text,
            voice: mapped_voice.as_deref(),
            mapped: &mapped,
            headers: BTreeMap::new(),
        });
        Ok(Self {
            mapped_voice,
            mapped,
            data,
        })
    }
}

fn voice_view(prepared: Prepared) -> HelperResult {
    Ok(json!({
        "voice": prepared.mapped_voice,
        "ssml": prepared.data.ssml_body,
    }))
}

fn text_views(texts: &Texts) -> HelperResult {
    let optional = SpeechOptionalParams::default();
    let kwargs = SpeechKwargs::default();
    let mut bodies = Vec::with_capacity(texts.inputs.len());
    for text in &texts.inputs {
        bodies.push(
            Prepared::new(text, None, &optional, &kwargs)?
                .data
                .ssml_body,
        );
    }
    Ok(json!({ "bodies": bodies }))
}

fn speed_views(speeds: &Speeds) -> HelperResult {
    let kwargs = SpeechKwargs::default();
    let mut views = Vec::with_capacity(speeds.speeds.len());
    for speed in &speeds.speeds {
        let optional = SpeechOptionalParams {
            response_format: None,
            speed: *speed,
        };
        let prepared = Prepared::new("hi", None, &optional, &kwargs)?;
        views.push(json!({
            "rate": prepared.mapped.rate,
            "ssml": prepared.data.ssml_body,
        }));
    }
    Ok(json!({ "speeds": views }))
}

fn voice_views(cases: &VoiceCases) -> HelperResult {
    let optional = SpeechOptionalParams::default();
    let mut views = Vec::with_capacity(cases.cases.len());
    for case in &cases.cases {
        let kwargs = SpeechKwargs {
            style: None,
            styledegree: None,
            role: None,
            lang: case.lang.clone(),
        };
        views.push(voice_view(Prepared::new(
            "hi",
            case.voice.as_ref(),
            &optional,
            &kwargs,
        )?)?);
    }
    Ok(json!({ "voices": views }))
}

fn style_views(cases: &StyleCases) -> HelperResult {
    let optional = SpeechOptionalParams::default();
    let mut views = Vec::with_capacity(cases.cases.len());
    for case in &cases.cases {
        let kwargs = SpeechKwargs {
            style: case.style.clone(),
            styledegree: case.styledegree.clone(),
            role: case.role.clone(),
            lang: case.lang.clone(),
        };
        let prepared = Prepared::new("hi", None, &optional, &kwargs)?;
        views.push(json!({
            "style": prepared.mapped.style,
            "styledegree": prepared.mapped.styledegree,
            "role": prepared.mapped.role,
            "lang": prepared.mapped.lang,
            "ssml": prepared.data.ssml_body,
        }));
    }
    Ok(json!({ "cases": views }))
}

fn ssml_views(texts: &Texts) -> HelperResult {
    let optional = SpeechOptionalParams::default();
    let kwargs = SpeechKwargs::default();
    let mut views = Vec::with_capacity(texts.inputs.len());
    for text in &texts.inputs {
        let prepared = Prepared::new(text, None, &optional, &kwargs)?;
        views.push(json!({
            "ssml": prepared.data.ssml_body,
            "header": prepared.data.headers.get("X-Microsoft-OutputFormat"),
        }));
    }
    Ok(json!({ "inputs": views }))
}

fn endpoint_views(bases: &Bases) -> HelperResult {
    let mut results = Vec::with_capacity(bases.api_bases.len());
    for api_base in &bases.api_bases {
        let identity = NativeSpeechIdentity {
            api_base: api_base.clone(),
            api_key: None,
        };
        match get_complete_url(&identity) {
            Ok(url) => results.push(json!({ "url": url })),
            Err(_) => results.push(json!({ "error": true })),
        }
    }
    Ok(json!({ "results": results }))
}

fn format_views(formats: &Formats) -> HelperResult {
    let kwargs = SpeechKwargs::default();
    let mut views = Vec::with_capacity(formats.formats.len());
    for format in &formats.formats {
        let optional = SpeechOptionalParams {
            response_format: format.clone(),
            speed: None,
        };
        let prepared = Prepared::new("hi", None, &optional, &kwargs)?;
        views.push(json!({
            "output_format": prepared.mapped.output_format,
            "header": prepared.data.headers.get("X-Microsoft-OutputFormat"),
        }));
    }
    Ok(json!({ "formats": views }))
}

fn binary_views(cases: &BinaryCases) -> HelperResult {
    let mut views = Vec::with_capacity(cases.cases.len());
    for case in &cases.cases {
        let mut headers = BTreeMap::new();
        if let Some(content_type) = &case.content_type {
            headers.insert("content-type".to_owned(), content_type.clone());
        }
        let raw = RawSpeechResponse {
            status: case.status,
            headers,
            body: case.body.as_bytes().to_vec(),
        };
        let content = HttpxBinaryResponseContent::from(raw);
        views.push(json!({
            "status": content.status,
            "contentType": content.headers.get("content-type"),
            "bytes": content.body,
        }));
    }
    Ok(json!({ "cases": views }))
}

fn voice_map_views(voices: &Voices) -> HelperResult {
    let optional = SpeechOptionalParams::default();
    let kwargs = SpeechKwargs::default();
    let mut views = Vec::with_capacity(voices.voices.len());
    for voice in &voices.voices {
        views.push(voice_view(Prepared::new(
            "hi",
            Some(voice),
            &optional,
            &kwargs,
        )?)?);
    }
    Ok(json!({ "voices": views }))
}

fn header_views(cases: &HeaderCases) -> HelperResult {
    let mut views = Vec::with_capacity(cases.cases.len());
    for case in &cases.cases {
        let identity = NativeSpeechIdentity {
            api_base: None,
            api_key: case.api_key.clone().map(SecretString::from),
        };
        let headers = validate_environment(&identity, case.headers.clone());
        views.push(json!({ "headers": headers }));
    }
    Ok(json!({ "cases": views }))
}

pub(super) fn invoke(input: &Input) -> HelperResult {
    match input {
        Input::PlainTextSsmlEscaping(texts) => text_views(texts),
        Input::SpeedRateBoundaries(speeds) => speed_views(speeds),
        Input::VoiceLanguage(cases) => voice_views(cases),
        Input::StyleRoleElement(cases) => style_views(cases),
        Input::ExistingSsml(texts) => ssml_views(texts),
        Input::CognitiveToTtsEndpoint(bases) => endpoint_views(bases),
        Input::ResponseFormatHeader(formats) => format_views(formats),
        Input::BinaryNativeSpeech(cases) => binary_views(cases),
        Input::NativeVoiceMap(voices) => voice_map_views(voices),
        Input::NativeSpeechHeaders(cases) => header_views(cases),
    }
}

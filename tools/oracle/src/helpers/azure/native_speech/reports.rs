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
    Bases, BinaryCases, Formats, HeaderCases, Input, Speeds, StyleCase, StyleCases, Texts,
    VoiceCases, Voices,
};

const NO_OPTIONS: SpeechOptionalParams = SpeechOptionalParams {
    response_format: None,
    speed: None,
};
const NO_KWARGS: SpeechKwargs = SpeechKwargs {
    style: None,
    styledegree: None,
    role: None,
    lang: None,
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

    fn styled(text: &str, case: &StyleCase) -> Result<Self, Box<HelperError>> {
        let kwargs = SpeechKwargs {
            style: case.style.clone(),
            styledegree: case.styledegree.clone(),
            role: case.role.clone(),
            lang: case.lang.clone(),
        };
        Self::new(text, None, &NO_OPTIONS, &kwargs)
    }
}

fn rows<T, F>(cases: &[T], mut view: F) -> HelperResult
where
    F: FnMut(&T) -> HelperResult,
{
    let mut rows = Vec::with_capacity(cases.len());
    for case in cases {
        rows.push(view(case)?);
    }
    Ok(json!(rows))
}

fn voice_row(voice: Option<&Json>, lang: Option<String>) -> HelperResult {
    let kwargs = SpeechKwargs {
        style: None,
        styledegree: None,
        role: None,
        lang,
    };
    let prepared = Prepared::new("hi", voice, &NO_OPTIONS, &kwargs)?;
    Ok(json!({
        "voice": prepared.mapped_voice,
        "ssml": prepared.data.ssml_body,
    }))
}

fn text_row(text: &str) -> HelperResult {
    Ok(json!(
        Prepared::new(text, None, &NO_OPTIONS, &NO_KWARGS)?
            .data
            .ssml_body
    ))
}

fn speed_row(speed: Option<f64>) -> HelperResult {
    let optional = SpeechOptionalParams {
        response_format: None,
        speed,
    };
    let prepared = Prepared::new("hi", None, &optional, &NO_KWARGS)?;
    Ok(json!({ "rate": prepared.mapped.rate, "ssml": prepared.data.ssml_body }))
}

fn style_row(case: &StyleCase) -> HelperResult {
    let prepared = Prepared::styled("hi", case)?;
    Ok(json!({
        "style": prepared.mapped.style,
        "styledegree": prepared.mapped.styledegree,
        "role": prepared.mapped.role,
        "lang": prepared.mapped.lang,
        "ssml": prepared.data.ssml_body,
    }))
}

fn ssml_row(text: &str) -> HelperResult {
    let prepared = Prepared::new(text, None, &NO_OPTIONS, &NO_KWARGS)?;
    Ok(json!({
        "ssml": prepared.data.ssml_body,
        "header": prepared.data.headers.get("X-Microsoft-OutputFormat"),
    }))
}

fn text_views(texts: &Texts) -> HelperResult {
    Ok(json!({
        "bodies": rows(&texts.inputs, |text| text_row(text.as_str()))?,
    }))
}

fn speed_views(speeds: &Speeds) -> HelperResult {
    Ok(json!({
        "speeds": rows(&speeds.speeds, |speed| speed_row(*speed))?,
    }))
}

fn voice_views(cases: &VoiceCases) -> HelperResult {
    Ok(json!({
        "voices": rows(&cases.cases, |case| {
            voice_row(case.voice.as_ref(), case.lang.clone())
        })?,
    }))
}

fn style_views(cases: &StyleCases) -> HelperResult {
    Ok(json!({
        "cases": rows(&cases.cases, style_row)?,
    }))
}

fn ssml_views(texts: &Texts) -> HelperResult {
    Ok(json!({
        "inputs": rows(&texts.inputs, |text| ssml_row(text.as_str()))?,
    }))
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

fn format_row(format: Option<&str>) -> HelperResult {
    let optional = SpeechOptionalParams {
        response_format: format.map(str::to_owned),
        speed: None,
    };
    let prepared = Prepared::new("hi", None, &optional, &NO_KWARGS)?;
    Ok(json!({
        "output_format": prepared.mapped.output_format,
        "header": prepared.data.headers.get("X-Microsoft-OutputFormat"),
    }))
}

fn format_views(formats: &Formats) -> HelperResult {
    Ok(json!({
        "formats": rows(&formats.formats, |format| format_row(format.as_deref()))?,
    }))
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
    Ok(json!({
        "voices": rows(&voices.voices, |voice| voice_row(Some(voice), None))?,
    }))
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

pub(super) fn build_answer(input: &Input) -> HelperResult {
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

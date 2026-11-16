use std::collections::BTreeMap;

use crate::native_speech::constants::DEFAULT_VOICE;
use crate::native_speech::mapping::MappedParams;
use crate::native_speech::ssml::build_ssml;

pub struct SpeechTransform<'call> {
    pub input: &'call str,
    pub voice: Option<&'call str>,
    pub mapped: &'call MappedParams,
    pub headers: BTreeMap<String, String>,
}

pub struct SpeechRequestData {
    pub ssml_body: String,
    pub headers: BTreeMap<String, String>,
}

#[must_use]
pub fn transform_request(transform: SpeechTransform<'_>) -> SpeechRequestData {
    let SpeechTransform {
        input,
        voice,
        mapped,
        mut headers,
    } = transform;
    headers.insert(
        "X-Microsoft-OutputFormat".to_owned(),
        mapped.output_format.clone(),
    );
    if input.contains("<speak>") || input.contains("<speak ") {
        return SpeechRequestData {
            ssml_body: input.to_owned(),
            headers,
        };
    }
    let spoken = voice
        .filter(|name| !name.is_empty())
        .unwrap_or(DEFAULT_VOICE);
    SpeechRequestData {
        ssml_body: build_ssml(input, spoken, mapped),
        headers,
    }
}

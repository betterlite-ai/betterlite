mod reports;
mod tests;

use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Texts {
    inputs: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Speeds {
    speeds: Vec<Option<f64>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VoiceCase {
    #[serde(default)]
    voice: Option<Json>,
    #[serde(default)]
    lang: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VoiceCases {
    cases: Vec<VoiceCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StyleCase {
    #[serde(default)]
    style: Option<String>,
    #[serde(default)]
    styledegree: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    lang: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StyleCases {
    cases: Vec<StyleCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bases {
    api_bases: Vec<Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Formats {
    formats: Vec<Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BinaryCase {
    status: u16,
    body: String,
    #[serde(default)]
    content_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BinaryCases {
    cases: Vec<BinaryCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Voices {
    voices: Vec<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderCase {
    headers: BTreeMap<String, String>,
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderCases {
    cases: Vec<HeaderCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    PlainTextSsmlEscaping(Texts),
    SpeedRateBoundaries(Speeds),
    VoiceLanguage(VoiceCases),
    StyleRoleElement(StyleCases),
    ExistingSsml(Texts),
    CognitiveToTtsEndpoint(Bases),
    ResponseFormatHeader(Formats),
    BinaryNativeSpeech(BinaryCases),
    NativeVoiceMap(Voices),
    NativeSpeechHeaders(HeaderCases),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        reports::build_answer(input)
    }
}

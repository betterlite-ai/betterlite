use betterlite_http::url_path::encode_segment;
use betterlite_openai::openai_base_config::Object;
use url::form_urlencoded;

use super::error::SpeechError;
use super::mapping::{QUERY_PARAMS_KEY, VOICE_ID_KEY};
use super::voice::STRIP_SPACE;

const TTS_ENDPOINT_PATH: &str = "/v1/text-to-speech";

fn query_string(query: &Object) -> Result<String, SpeechError> {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (name, value) in query {
        let text = value
            .as_str()
            .map(str::to_owned)
            .or_else(|| value.is_number().then(|| value.to_string()))
            .ok_or_else(|| SpeechError::QueryValue { name: name.clone() })?;
        serializer.append_pair(name, &text);
    }
    // Python's quote_plus leaves ~ alone and encodes *, the reverse of form_urlencoded
    Ok(serializer.finish().replace('*', "%2A").replace("%7E", "~"))
}

pub fn complete_url(
    api_base: Option<&str>,
    environment_base: &str,
    litellm_params: &Object,
) -> Result<String, SpeechError> {
    let base = api_base
        .filter(|base| !base.is_empty())
        .unwrap_or(environment_base)
        .trim_end_matches('/');
    let voice_id = litellm_params
        .get(VOICE_ID_KEY)
        .ok_or(SpeechError::VoiceRequired)?
        .as_str()
        .filter(|text| !text.trim_matches(STRIP_SPACE).is_empty())
        .ok_or(SpeechError::VoiceRequired)?;
    let encoded = encode_segment(voice_id, "voice_id")
        .map_err(|source| SpeechError::VoiceSegment { source })?;
    let url = format!("{base}{TTS_ENDPOINT_PATH}/{encoded}");
    let Some(query_value) = litellm_params
        .get(QUERY_PARAMS_KEY)
        .filter(|value| !value.is_null())
    else {
        return Ok(url);
    };
    let query = query_value.as_object().ok_or(SpeechError::QueryShape)?;
    if query.is_empty() {
        return Ok(url);
    }
    Ok(format!("{url}?{}", query_string(query)?))
}

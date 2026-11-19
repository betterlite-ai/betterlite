use betterlite_openai::api::audio_file_input::decoder::AudioDecodeLimits;
use betterlite_openai::api::speech_chat_bridge::media_type::{
    resolve_speech_media_type, sniff_speech_audio,
};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::Value;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
struct Input {
    resolve: Vec<ResolveCase>,
    sniff: Vec<SniffCase>,
}

// the upstream Content-Type and requested format a route hands the resolver
#[derive(Debug, Deserialize)]
struct ResolveCase {
    #[serde(default)]
    upstream: Option<String>,
    #[serde(default)]
    response_format: Option<String>,
}

// one provider audio body, as the bytes of its header
#[derive(Debug, Deserialize)]
struct SniffCase {
    audio: Vec<u8>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let resolve: Vec<Value> = input
            .resolve
            .iter()
            .map(|case| {
                Value::String(resolve_speech_media_type(
                    case.upstream.as_deref(),
                    case.response_format.as_deref(),
                ))
            })
            .collect();
        let sniff: Vec<Value> = input
            .sniff
            .iter()
            .map(
                |case| match sniff_speech_audio(&case.audio, &AudioDecodeLimits::default()) {
                    Some(media_type) => Value::String(media_type.to_owned()),
                    None => Value::Null,
                },
            )
            .collect();
        Ok(serde_json::json!({ "resolve": resolve, "sniff": sniff }))
    }
}

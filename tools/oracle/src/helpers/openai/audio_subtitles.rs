use betterlite_openai::api::audio_subtitles::SubtitleToken;
use betterlite_openai::api::audio_subtitles::grouping::group_tokens_into_cues;
use betterlite_openai::api::audio_subtitles::render::{
    render_subtitle_tokens_as_srt, render_subtitle_tokens_as_vtt,
};
use betterlite_openai::api::audio_subtitles::timing::synthesize_subtitle_document;
use serde::Deserialize;
use serde_json::{Value, json};

mod tests;

#[derive(Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Group {
        tokens: Vec<SubtitleToken>,
    },
    Srt {
        tokens: Vec<SubtitleToken>,
    },
    Vtt {
        tokens: Vec<SubtitleToken>,
    },
    Synthesize {
        words: Value,
        response_format: String,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outcomes: Vec<Value> = input.calls.iter().map(outcome_of).collect();
        Ok(serde_json::to_value(outcomes)?)
    }
}

fn outcome_of(call: &Call) -> Value {
    match call {
        Call::Group { tokens } => {
            json!({"ok": true, "value": group_tokens_into_cues(tokens)})
        }
        Call::Srt { tokens } => json!({"ok": true, "value": render_subtitle_tokens_as_srt(tokens)}),
        Call::Vtt { tokens } => json!({"ok": true, "value": render_subtitle_tokens_as_vtt(tokens)}),
        Call::Synthesize {
            words,
            response_format,
        } => synthesis_outcome(words, response_format),
    }
}

fn synthesis_outcome(words: &Value, response_format: &str) -> Value {
    match synthesize_subtitle_document(words, response_format) {
        Ok(document) => json!({"ok": true, "value": document}),
        Err(_error) => json!({"ok": false}),
    }
}
